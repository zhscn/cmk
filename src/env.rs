use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::Path,
    process::Command,
};

const CONFIG_FILE_NAME: &str = ".cmk.toml";

/// Represents an environment variable value that can be:
/// - A simple string (set directly)
/// - A path modification (prepend or append)
#[derive(Debug, Clone)]
pub enum EnvValue {
    Set(String),
    Prepend(Vec<String>),
    Append(Vec<String>),
}

impl EnvValue {
    /// Resolve the final value, optionally merging with existing env var
    pub fn resolve(&self, existing: Option<&str>) -> String {
        match self {
            EnvValue::Set(v) => v.clone(),
            EnvValue::Prepend(paths) => {
                let new_paths = paths.join(":");
                match existing {
                    Some(existing) if !existing.is_empty() => format!("{new_paths}:{existing}"),
                    _ => new_paths,
                }
            }
            EnvValue::Append(paths) => {
                let new_paths = paths.join(":");
                match existing {
                    Some(existing) if !existing.is_empty() => format!("{existing}:{new_paths}"),
                    _ => new_paths,
                }
            }
        }
    }
}

/// Raw TOML structure for environment variable value
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawEnvValue {
    Simple(String),
    PathMod(PathModifier),
}

#[derive(Debug, Deserialize)]
struct PathModifier {
    #[serde(default)]
    prepend: Option<Vec<String>>,
    #[serde(default)]
    append: Option<Vec<String>>,
}

impl From<RawEnvValue> for EnvValue {
    fn from(raw: RawEnvValue) -> Self {
        match raw {
            RawEnvValue::Simple(s) => EnvValue::Set(s),
            RawEnvValue::PathMod(m) => {
                if let Some(paths) = m.prepend {
                    EnvValue::Prepend(paths)
                } else if let Some(paths) = m.append {
                    EnvValue::Append(paths)
                } else {
                    EnvValue::Set(String::new())
                }
            }
        }
    }
}

/// Raw TOML configuration structure
#[derive(Debug, Deserialize, Default)]
struct RawEnvConfig {
    #[serde(default)]
    vars: HashMap<String, String>,
    #[serde(default)]
    env: HashMap<String, toml::Value>,
}

/// Parsed environment configuration
#[derive(Debug, Default)]
pub struct EnvConfig {
    /// Custom variables for expansion
    vars: HashMap<String, String>,
    /// Common environment for all commands
    common: HashMap<String, EnvValue>,
    /// Build-specific environment
    build: HashMap<String, EnvValue>,
    /// Default run environment
    run: HashMap<String, EnvValue>,
    /// Target-specific run environment
    run_targets: HashMap<String, HashMap<String, EnvValue>>,
    /// Linux-specific environment
    linux: HashMap<String, EnvValue>,
    /// macOS-specific environment
    macos: HashMap<String, EnvValue>,
}

impl EnvConfig {
    /// Load configuration from project root
    pub fn load(project_root: &Path) -> Result<Self> {
        let config_path = project_root.join(CONFIG_FILE_NAME);
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;

        Self::parse(&content, project_root)
    }

    /// Parse TOML content into EnvConfig
    fn parse(content: &str, project_root: &Path) -> Result<Self> {
        let raw: RawEnvConfig = toml::from_str(content)?;

        let mut config = EnvConfig {
            vars: raw.vars,
            ..Default::default()
        };

        // Add PROJECT_ROOT as a built-in variable
        config
            .vars
            .insert("PROJECT_ROOT".to_string(), project_root.display().to_string());

        // Parse the env section
        for (key, value) in raw.env {
            match key.as_str() {
                "build" => {
                    config.build = Self::parse_env_table(value)?;
                }
                "run" => {
                    config.run_targets = Self::parse_run_section(value, &mut config.run)?;
                }
                "linux" => {
                    config.linux = Self::parse_env_table(value)?;
                }
                "macos" => {
                    config.macos = Self::parse_env_table(value)?;
                }
                _ => {
                    // Common environment variable
                    config.common.insert(key, Self::parse_env_value(value)?);
                }
            }
        }

        Ok(config)
    }

    /// Parse a table of environment variables
    fn parse_env_table(value: toml::Value) -> Result<HashMap<String, EnvValue>> {
        let mut result = HashMap::new();
        if let toml::Value::Table(table) = value {
            for (k, v) in table {
                result.insert(k, Self::parse_env_value(v)?);
            }
        }
        Ok(result)
    }

    /// Parse the run section which may contain both default env and target-specific env
    fn parse_run_section(
        value: toml::Value,
        default_run: &mut HashMap<String, EnvValue>,
    ) -> Result<HashMap<String, HashMap<String, EnvValue>>> {
        let mut targets = HashMap::new();

        if let toml::Value::Table(table) = value {
            for (k, v) in table {
                if let toml::Value::Table(_) = &v {
                    // Check if it looks like an env var definition or a target section
                    if Self::is_target_section(&v) {
                        targets.insert(k, Self::parse_env_table(v)?);
                    } else {
                        default_run.insert(k, Self::parse_env_value(v)?);
                    }
                } else {
                    default_run.insert(k, Self::parse_env_value(v)?);
                }
            }
        }

        Ok(targets)
    }

    /// Check if a value represents a target section (contains env var definitions)
    fn is_target_section(value: &toml::Value) -> bool {
        if let toml::Value::Table(table) = value {
            // If the table contains "prepend" or "append", it's a path modifier, not a target
            if table.contains_key("prepend") || table.contains_key("append") {
                return false;
            }
            // Otherwise, assume it's a target section if it contains any entries
            !table.is_empty()
        } else {
            false
        }
    }

    /// Parse a single environment variable value
    fn parse_env_value(value: toml::Value) -> Result<EnvValue> {
        match value {
            toml::Value::String(s) => Ok(EnvValue::Set(s)),
            toml::Value::Table(table) => {
                if let Some(toml::Value::Array(arr)) = table.get("prepend") {
                    let paths: Result<Vec<String>, _> = arr
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .ok_or_else(|| anyhow::anyhow!("Expected string in prepend array"))
                        })
                        .collect();
                    Ok(EnvValue::Prepend(paths?))
                } else if let Some(toml::Value::Array(arr)) = table.get("append") {
                    let paths: Result<Vec<String>, _> = arr
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .ok_or_else(|| anyhow::anyhow!("Expected string in append array"))
                        })
                        .collect();
                    Ok(EnvValue::Append(paths?))
                } else {
                    Ok(EnvValue::Set(String::new()))
                }
            }
            toml::Value::Array(arr) => {
                // Default array behavior is prepend
                let paths: Result<Vec<String>, _> = arr
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| anyhow::anyhow!("Expected string in array"))
                    })
                    .collect();
                Ok(EnvValue::Prepend(paths?))
            }
            _ => Ok(EnvValue::Set(String::new())),
        }
    }

    /// Expand variables in a string (${VAR} syntax)
    fn expand_vars(&self, s: &str, build_dir: Option<&Path>) -> String {
        let mut result = s.to_string();

        // Handle PROJECT_BUILD_ROOT if build_dir is provided
        if let Some(dir) = build_dir {
            let pattern = "${PROJECT_BUILD_ROOT}";
            if result.contains(pattern) {
                result = result.replace(pattern, &dir.display().to_string());
            }
        }

        // Keep expanding until no more variables are found (handles nested vars)
        loop {
            let mut changed = false;
            for (var, value) in &self.vars {
                let pattern = format!("${{{var}}}");
                if result.contains(&pattern) {
                    result = result.replace(&pattern, value);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        result
    }

    /// Expand variables in an EnvValue
    fn expand_env_value(&self, value: &EnvValue, build_dir: Option<&Path>) -> EnvValue {
        match value {
            EnvValue::Set(s) => EnvValue::Set(self.expand_vars(s, build_dir)),
            EnvValue::Prepend(paths) => {
                EnvValue::Prepend(paths.iter().map(|p| self.expand_vars(p, build_dir)).collect())
            }
            EnvValue::Append(paths) => {
                EnvValue::Append(paths.iter().map(|p| self.expand_vars(p, build_dir)).collect())
            }
        }
    }

    /// Get platform-specific environment
    fn platform_env(&self) -> &HashMap<String, EnvValue> {
        if cfg!(target_os = "macos") {
            &self.macos
        } else {
            &self.linux
        }
    }

    /// Build environment for build commands (cmake, ninja)
    pub fn build_env(&self, build_dir: Option<&Path>) -> HashMap<String, String> {
        let mut result = HashMap::new();

        // Layer: common -> platform -> build
        self.apply_layer(&mut result, &self.common, build_dir);
        self.apply_layer(&mut result, self.platform_env(), build_dir);
        self.apply_layer(&mut result, &self.build, build_dir);

        result
    }

    /// Build environment for running a target
    pub fn run_env(&self, target_name: Option<&str>, build_dir: Option<&Path>) -> HashMap<String, String> {
        let mut result = HashMap::new();

        // Layer: common -> platform -> run -> target-specific
        self.apply_layer(&mut result, &self.common, build_dir);
        self.apply_layer(&mut result, self.platform_env(), build_dir);
        self.apply_layer(&mut result, &self.run, build_dir);

        if let Some(name) = target_name {
            if let Some(target_env) = self.run_targets.get(name) {
                self.apply_layer(&mut result, target_env, build_dir);
            }
        }

        result
    }

    /// Apply a layer of environment variables to the result
    fn apply_layer(
        &self,
        result: &mut HashMap<String, String>,
        layer: &HashMap<String, EnvValue>,
        build_dir: Option<&Path>,
    ) {
        for (key, value) in layer {
            let expanded = self.expand_env_value(value, build_dir);

            // Get existing value from result or from actual environment
            let existing_val = result
                .get(key)
                .cloned()
                .or_else(|| std::env::var(key).ok());

            let resolved = expanded.resolve(existing_val.as_deref());
            result.insert(key.clone(), resolved);
        }
    }

    /// Apply environment variables to a Command
    pub fn apply_to_command(&self, cmd: &mut Command, env: &HashMap<String, String>) {
        for (key, value) in env {
            cmd.env(key, value);
        }
    }

    /// Check if config file exists
    pub fn exists(project_root: &Path) -> bool {
        project_root.join(CONFIG_FILE_NAME).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_simple_config() {
        let content = r#"
[vars]
DEPS_DIR = "${PROJECT_ROOT}/.deps"
DEPS_INSTALL = "${DEPS_DIR}/install"

[env]
CC = "clang"
PATH = { prepend = ["${DEPS_INSTALL}/bin"] }

[env.build]
CXX = "clang++"

[env.run]
LD_LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib"] }

[env.run.my_target]
MY_VAR = "test_value"

[env.linux]
LD_LIBRARY_PATH = { prepend = ["${DEPS_INSTALL}/lib", "${DEPS_INSTALL}/lib64"] }
"#;

        let project_root = PathBuf::from("/test/project");
        let config = EnvConfig::parse(content, &project_root).unwrap();

        assert_eq!(config.vars.get("DEPS_DIR").unwrap(), "${PROJECT_ROOT}/.deps");
        assert!(config.common.contains_key("CC"));
        assert!(config.common.contains_key("PATH"));
        assert!(config.build.contains_key("CXX"));
        assert!(config.run.contains_key("LD_LIBRARY_PATH"));
        assert!(config.run_targets.contains_key("my_target"));
        assert!(config.linux.contains_key("LD_LIBRARY_PATH"));
    }

    #[test]
    fn test_expand_vars() {
        let mut config = EnvConfig::default();
        config.vars.insert("PROJECT_ROOT".to_string(), "/test".to_string());
        config.vars.insert("DEPS_DIR".to_string(), "${PROJECT_ROOT}/.deps".to_string());

        // First level expansion
        let result = config.expand_vars("${PROJECT_ROOT}/bin", None);
        assert_eq!(result, "/test/bin");

        // Test PROJECT_BUILD_ROOT expansion
        let build_dir = PathBuf::from("/test/build");
        let result = config.expand_vars("${PROJECT_BUILD_ROOT}/lib", Some(&build_dir));
        assert_eq!(result, "/test/build/lib");
    }

    #[test]
    fn test_env_value_resolve() {
        let prepend = EnvValue::Prepend(vec!["/new/path".to_string()]);
        assert_eq!(prepend.resolve(Some("/existing")), "/new/path:/existing");
        assert_eq!(prepend.resolve(None), "/new/path");

        let append = EnvValue::Append(vec!["/new/path".to_string()]);
        assert_eq!(append.resolve(Some("/existing")), "/existing:/new/path");

        let set = EnvValue::Set("value".to_string());
        assert_eq!(set.resolve(Some("/existing")), "value");
    }
}
