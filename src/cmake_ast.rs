use anyhow::{Context, Result};
use std::{
    ops::Range,
    path::{Path, PathBuf},
};
use tree_sitter::{Node, Parser, Tree};

/// A parsed CMakeLists.txt file with mutable source. Edits are applied via
/// `splice` (callers should sort edits by descending start byte to keep
/// offsets valid), and the tree is lazily re-parsed only when re-walking.
pub struct CMakeFile {
    pub path: PathBuf,
    pub source: String,
    tree: Tree,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub struct CpmCall {
    /// Byte range of the entire `CPMAddPackage(...)` invocation, including
    /// the trailing `)`.
    pub call_range: Range<usize>,
    /// Parsed URI form, when the call has the shape
    /// `CPMAddPackage("<source>:<owner>/<repo>(#|@)<version>")`.
    /// Named-argument forms parse to `None` for v1.
    pub uri: Option<CpmUri>,
}

#[derive(Debug, Clone)]
pub struct CpmUri {
    /// Shorthand source: `gh`, `gl`, `bb`.
    pub source: String,
    pub owner: String,
    pub repo: String,
    /// Version pin, when present.
    pub version: Option<String>,
    /// `#` or `@`, when version is present.
    pub version_separator: Option<char>,
    /// Byte range within the file of just the version string (without
    /// the leading `#`/`@`), suitable for `splice`. None when the URI
    /// has no version pin.
    pub version_range: Option<Range<usize>>,
    /// Byte range of the URI text inside the quotes (so the content
    /// `gh:owner/repo#v1` excluding the surrounding `"`).
    pub uri_content_range: Range<usize>,
}

fn parse_source(source: &str) -> Result<Tree> {
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_cmake::LANGUAGE.into();
    parser
        .set_language(&lang)
        .with_context(|| "Failed to load tree-sitter CMake grammar")?;
    parser
        .parse(source, None)
        .with_context(|| "Tree-sitter failed to parse CMakeLists.txt")
}

impl CMakeFile {
    pub fn parse_path(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        Self::from_source(source, path.to_path_buf())
    }

    pub fn from_source(source: String, path: PathBuf) -> Result<Self> {
        let tree = parse_source(&source)?;
        Ok(Self {
            path,
            source,
            tree,
            dirty: false,
        })
    }

    /// Re-parse the source if it has been edited since the last walk.
    fn ensure_fresh(&mut self) {
        if self.dirty
            && let Ok(tree) = parse_source(&self.source)
        {
            self.tree = tree;
            self.dirty = false;
        }
    }

    /// Walk all `normal_command` nodes with name `CPMAddPackage`
    /// (case-insensitive — CMake commands are case-insensitive).
    pub fn cpm_calls(&mut self) -> Vec<CpmCall> {
        self.ensure_fresh();
        let mut out = Vec::new();
        let root = self.tree.root_node();
        let src = self.source.as_bytes();
        for cmd in iter_children(root) {
            if cmd.kind() != "normal_command" {
                continue;
            }
            let Some(name_node) = cmd.named_child(0) else {
                continue;
            };
            if name_node.kind() != "identifier" {
                continue;
            }
            let name = name_node.utf8_text(src).unwrap_or("");
            if !name.eq_ignore_ascii_case("CPMAddPackage") {
                continue;
            }
            out.push(parse_cpm_call(cmd, src));
        }
        out
    }

    /// Replace `range` with `replacement` in the source. Marks the tree
    /// as dirty so the next walker call re-parses.
    pub fn splice(&mut self, range: Range<usize>, replacement: &str) {
        self.source.replace_range(range, replacement);
        self.dirty = true;
    }

    /// Apply a batch of edits in one shot, sorted by descending start byte
    /// so positions remain valid as we go.
    pub fn splice_many(&mut self, mut edits: Vec<(Range<usize>, String)>) {
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        for (range, replacement) in edits {
            self.source.replace_range(range, &replacement);
        }
        self.dirty = true;
    }

    /// Find a sensible insertion byte offset for a new top-level command.
    /// Strategy: after the last `CPMAddPackage` call, otherwise just before
    /// the first `add_executable`/`add_library`, otherwise end of file.
    /// Returns the insertion offset (callers prepend/append their own
    /// newline as appropriate).
    pub fn cpm_insertion_offset(&mut self) -> usize {
        self.ensure_fresh();
        let root = self.tree.root_node();
        let src = self.source.as_bytes();
        let mut last_cpm_end: Option<usize> = None;
        let mut first_target_start: Option<usize> = None;
        for cmd in iter_children(root) {
            if cmd.kind() != "normal_command" {
                continue;
            }
            let Some(name_node) = cmd.named_child(0) else {
                continue;
            };
            if name_node.kind() != "identifier" {
                continue;
            }
            let name = name_node.utf8_text(src).unwrap_or("");
            if name.eq_ignore_ascii_case("CPMAddPackage") {
                last_cpm_end = Some(cmd.end_byte());
            } else if first_target_start.is_none()
                && (name.eq_ignore_ascii_case("add_executable")
                    || name.eq_ignore_ascii_case("add_library"))
            {
                first_target_start = Some(cmd.start_byte());
            }
        }
        if let Some(end) = last_cpm_end {
            return end;
        }
        if let Some(start) = first_target_start {
            return start;
        }
        self.source.len()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::write(&self.path, &self.source)
            .with_context(|| format!("Failed to write {}", self.path.display()))
    }
}

fn iter_children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    let mut walker = node.walk();
    let mut children = Vec::new();
    if walker.goto_first_child() {
        loop {
            children.push(walker.node());
            if !walker.goto_next_sibling() {
                break;
            }
        }
    }
    children.into_iter()
}

fn parse_cpm_call(cmd: Node<'_>, src: &[u8]) -> CpmCall {
    let mut uri = None;
    if let Some(arg_list) = find_child(cmd, "argument_list") {
        let args: Vec<Node<'_>> = iter_children(arg_list)
            .filter(|n| n.kind() == "argument")
            .collect();
        if args.len() == 1
            && let Some(quoted) = find_child(args[0], "quoted_argument")
            && let Some(content) = find_child(quoted, "quoted_element")
        {
            let text = content.utf8_text(src).unwrap_or("");
            uri = parse_cpm_uri(text, content.start_byte()..content.end_byte());
        }
    }
    CpmCall {
        call_range: cmd.start_byte()..cmd.end_byte(),
        uri,
    }
}

fn find_child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    iter_children(node).find(|n| n.kind() == kind)
}

/// Parse `gh:owner/repo#version`, `gh:owner/repo@version`, `gh:owner/repo`.
/// `content_range` is the byte range of `text` within the source file.
fn parse_cpm_uri(text: &str, content_range: Range<usize>) -> Option<CpmUri> {
    let (source, rest) = text.split_once(':')?;
    if !matches!(source, "gh" | "gl" | "bb") {
        return None;
    }
    let (owner, after_owner) = rest.split_once('/')?;
    if owner.is_empty() {
        return None;
    }
    let mut repo_end = after_owner.len();
    let mut sep_idx: Option<usize> = None;
    let mut sep_char: Option<char> = None;
    for (i, c) in after_owner.char_indices() {
        if c == '#' || c == '@' {
            repo_end = i;
            sep_idx = Some(i);
            sep_char = Some(c);
            break;
        }
    }
    let repo = &after_owner[..repo_end];
    if repo.is_empty() {
        return None;
    }

    // Compute byte offsets relative to the file. `text` corresponds to
    // `content_range`; offsets within `text` map directly because the URI
    // is ASCII-only by convention (and we use byte indices throughout).
    let after_owner_offset_in_text = source.len() + 1 + owner.len() + 1;
    let (version, version_range) = if let Some(i) = sep_idx {
        let v = &after_owner[i + 1..];
        if v.is_empty() {
            (None, None)
        } else {
            let v_start = content_range.start + after_owner_offset_in_text + i + 1;
            let v_end = v_start + v.len();
            (Some(v.to_string()), Some(v_start..v_end))
        }
    } else {
        (None, None)
    };

    Some(CpmUri {
        source: source.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        version,
        version_separator: sep_char,
        version_range,
        uri_content_range: content_range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uri_with_hash_version() {
        let src = r#"CPMAddPackage("gh:fmtlib/fmt#12.1.0")
"#;
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let calls = f.cpm_calls();
        assert_eq!(calls.len(), 1);
        let uri = calls[0].uri.as_ref().unwrap();
        assert_eq!(uri.source, "gh");
        assert_eq!(uri.owner, "fmtlib");
        assert_eq!(uri.repo, "fmt");
        assert_eq!(uri.version.as_deref(), Some("12.1.0"));
        assert_eq!(uri.version_separator, Some('#'));
        let r = uri.version_range.clone().unwrap();
        assert_eq!(&src[r], "12.1.0");
    }

    #[test]
    fn parses_uri_with_at_version() {
        let src = r#"CPMAddPackage("gh:catchorg/Catch2@v3.5.4")
"#;
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let calls = f.cpm_calls();
        let uri = calls[0].uri.as_ref().unwrap();
        assert_eq!(uri.repo, "Catch2");
        assert_eq!(uri.version.as_deref(), Some("v3.5.4"));
        assert_eq!(uri.version_separator, Some('@'));
    }

    #[test]
    fn ignores_named_argument_form() {
        let src = r#"CPMAddPackage(
  NAME spdlog
  GITHUB_REPOSITORY gabime/spdlog
  GIT_TAG v1.14.1
)
"#;
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let calls = f.cpm_calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].uri.is_none());
    }

    #[test]
    fn case_insensitive_command_name() {
        let src = "cpmaddpackage(\"gh:foo/bar#1\")\n";
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let calls = f.cpm_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].uri.as_ref().unwrap().repo, "bar");
    }

    #[test]
    fn splice_updates_version_in_place() {
        let src = "CPMAddPackage(\"gh:fmtlib/fmt#12.1.0\")\n";
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let calls = f.cpm_calls();
        let r = calls[0].uri.as_ref().unwrap().version_range.clone().unwrap();
        f.splice(r, "12.2.0");
        assert!(f.source.contains("gh:fmtlib/fmt#12.2.0"));
    }

    #[test]
    fn insertion_after_last_cpm() {
        let src = "CPMAddPackage(\"gh:a/b#1\")\n\nadd_executable(x src/x.cc)\n";
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let off = f.cpm_insertion_offset();
        // end of `CPMAddPackage("gh:a/b#1")` is byte 25
        assert_eq!(&src[..off], "CPMAddPackage(\"gh:a/b#1\")");
    }

    #[test]
    fn insertion_before_first_target() {
        let src = "add_executable(x src/x.cc)\n";
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let off = f.cpm_insertion_offset();
        assert_eq!(off, 0);
    }

    #[test]
    fn insertion_falls_back_to_eof() {
        let src = "project(foo)\n";
        let mut f = CMakeFile::from_source(src.to_string(), PathBuf::from("test")).unwrap();
        let off = f.cpm_insertion_offset();
        assert_eq!(off, src.len());
    }
}
