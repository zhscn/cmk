use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};

use crate::completing_read;

pub enum Template {
    BuiltIn,
    Custom(PathBuf),
}

pub async fn load_template(template: Option<&str>) -> Result<Template> {
    let home = std::env::var("HOME")?;
    let templates_dir = Path::new(&home).join(".config/cmk/templates");

    const BUILTIN_NAME: &str = "builtin";

    if let Some(name) = template {
        if name == BUILTIN_NAME {
            return Ok(Template::BuiltIn);
        }
        let dir = templates_dir.join(name);
        if !dir.is_dir() {
            return Err(anyhow!(
                "Template '{}' not found at {}",
                name,
                dir.display()
            ));
        }
        return Ok(Template::Custom(dir));
    }

    if templates_dir.is_dir() {
        let mut entries: Vec<String> = std::fs::read_dir(&templates_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();

        if !entries.is_empty() {
            entries.insert(0, BUILTIN_NAME.to_string());
            let chosen = completing_read(&entries).await?;
            if chosen == BUILTIN_NAME {
                return Ok(Template::BuiltIn);
            }
            if !chosen.is_empty() {
                return Ok(Template::Custom(templates_dir.join(&chosen)));
            }
        }
    }

    Ok(Template::BuiltIn)
}

impl Template {
    pub fn apply(&self, project_dir: &Path, vars: &HashMap<&str, &str>) -> Result<()> {
        match self {
            Template::BuiltIn => {
                std::fs::create_dir_all(project_dir.join("src"))?;
                std::fs::write(project_dir.join(".gitignore"), GIT_IGNORE)?;
                std::fs::write(project_dir.join(".clang-format"), CLANG_FORMAT_CONFIG)?;
                std::fs::write(project_dir.join(".clang-tidy"), CLANG_TIDY_CONFIG)?;
                std::fs::write(project_dir.join("src/main.cc"), MAIN_CC)?;
                let cmake = substitute(CMAKE_LISTS, vars);
                std::fs::write(project_dir.join("CMakeLists.txt"), cmake)?;
                Ok(())
            }
            Template::Custom(template_dir) => copy_dir_recursive(template_dir, project_dir, vars),
        }
    }
}

fn substitute(content: &str, vars: &HashMap<&str, &str>) -> String {
    let mut result = content.to_string();
    for (key, value) in vars {
        result = result.replace(key, value);
    }
    result
}

fn copy_dir_recursive(src: &Path, dst: &Path, vars: &HashMap<&str, &str>) -> Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let rel = src_path.strip_prefix(src)?;
        let dst_path = dst.join(rel);

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path, vars)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = std::fs::read_to_string(&src_path)
                .with_context(|| format!("Failed to read template file: {}", src_path.display()))?;
            std::fs::write(&dst_path, substitute(&content, vars))?;
        }
    }
    Ok(())
}

pub const GIT_IGNORE: &str = r#"# Prerequisites
*.d

# Compiled Object files
*.slo
*.lo
*.o
*.obj

# Precompiled Headers
*.gch
*.pch

# Compiled Dynamic libraries
*.so
*.dylib
*.dll

# Fortran module files
*.mod
*.smod

# Compiled Static libraries
*.lai
*.la
*.a
*.lib

# Executables
*.exe
*.out
*.app

# LSP Cache
.ccls-cache
.cache
compile_commands.json

# Build Directory
build/
"#;

pub const CLANG_FORMAT_CONFIG: &str = r#"---
Language:        Cpp
BasedOnStyle:  Google
AccessModifierOffset: -2
IncludeBlocks: Preserve
IndentCaseLabels: false
PointerAlignment: Right
...
"#;

pub const CLANG_TIDY_CONFIG: &str = r#"---
Checks: '
        bugprone-*,
        clang-analyzer-*,
        cppcoreguidelines-*,
        modernize-*,
        performance-*,
        portability-*,
        readability-*,
        -bugprone-easily-swappable-parameters,
        -cppcoreguidelines-avoid-magic-numbers,
        -cppcoreguidelines-non-private-member-variables-in-classes,
        -cppcoreguidelines-pro-type-vararg,
        -modernize-use-nodiscard,
        -modernize-use-ranges,
        -modernize-use-trailing-return-type,
        -readability-identifier-length,
        -readability-function-cognitive-complexity,
        -readability-magic-numbers,
        -readability-math-missing-parentheses,
        -readability-qualified-auto,
        -readability-static-accessed-through-instance
        '

CheckOptions:
  - key: cppcoreguidelines-special-member-functions.AllowImplicitlyDeletedCopyOrMove
    value: true
"#;

pub const CMAKE_LISTS: &str = r#"cmake_minimum_required(VERSION 3.20)
project(
  {name}
  VERSION 0.1.0
  LANGUAGES CXX C
)

list(APPEND CMAKE_MODULE_PATH ${CMAKE_SOURCE_DIR}/cmake)

### Options
if(POLICY CMP0167)
  cmake_policy(SET CMP0167 NEW)
endif()

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_compile_options(-Wall -Wextra)

### CPM
set(CPM_DOWNLOAD_VERSION "{cpm_version}")
set(CPM_HASH_SUM "{cpm_hash_sum}")
set(CPM_DOWNLOAD_URL "https://github.com/cpm-cmake/CPM.cmake/releases/download/v${CPM_DOWNLOAD_VERSION}/CPM.cmake")

if(CPM_SOURCE_CACHE)
  set(CPM_DOWNLOAD_LOCATION "${CPM_SOURCE_CACHE}/cpm/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
elseif(DEFINED ENV{CPM_SOURCE_CACHE})
  set(CPM_DOWNLOAD_LOCATION "$ENV{CPM_SOURCE_CACHE}/cpm/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
else()
  set(CPM_DOWNLOAD_LOCATION "${CMAKE_BINARY_DIR}/cmake/CPM_${CPM_DOWNLOAD_VERSION}.cmake")
endif()

get_filename_component(CPM_DOWNLOAD_LOCATION ${CPM_DOWNLOAD_LOCATION} ABSOLUTE)

if (NOT EXISTS ${CPM_DOWNLOAD_LOCATION})
  file(DOWNLOAD ${CPM_DOWNLOAD_URL} ${CPM_DOWNLOAD_LOCATION}
       EXPECTED_HASH SHA256=${CPM_HASH_SUM})
endif()

include(${CPM_DOWNLOAD_LOCATION})

### Library
CPMAddPackage("gh:fmtlib/fmt#12.1.0")

### Executable
add_executable({name} src/main.cc)
target_link_libraries({name} PRIVATE fmt::fmt)
target_compile_options({name} PRIVATE $<$<CONFIG:Debug>:-fsanitize=address,undefined>)
target_link_options({name} PRIVATE $<$<CONFIG:Debug>:-fsanitize=address,undefined>)
"#;

pub const MAIN_CC: &str = r#"#include <fmt/format.h>

int main() {
    fmt::println("Hello, world!");
    return 0;
}
"#;
