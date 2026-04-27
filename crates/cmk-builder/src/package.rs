use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use cmk_toolchain::extract::sha256_file;

/// Tools that go into the `toolchain` package (design §3.1).
const TOOLCHAIN_BINS: &[&str] = &[
    "clang",
    "clang++",
    "clang-cpp",
    "lld",
    "ld.lld",
    "lld-link",
    "ld64.lld",
    "wasm-ld",
    "llvm-ar",
    "llvm-nm",
    "llvm-objcopy",
    "llvm-objdump",
    "llvm-ranlib",
    "llvm-strip",
    "llvm-readelf",
    "llvm-readobj",
    "llvm-symbolizer",
    "llvm-cov",
    "llvm-profdata",
    "llvm-config",
    "llvm-dwarfdump",
    "llvm-size",
    "llvm-otool",
];

/// Tools that go into the `tools-extra` package (design §3.1).
const TOOLS_EXTRA_BINS: &[&str] = &[
    "clang-tidy",
    "clangd",
    "clang-format",
    "clang-apply-replacements",
    "clang-include-cleaner",
    "clang-query",
    "clang-refactor",
    "clang-rename",
    "clang-doc",
    "clang-change-namespace",
    "clang-reorder-fields",
    "find-all-symbols",
    "modularize",
    "pp-trace",
    "clang-check",
    "clang-extdef-mapping",
    "clang-installapi",
    "clang-linker-wrapper",
    "clang-nvlink-wrapper",
    "clang-offload-bundler",
    "clang-offload-packager",
    "clang-pseudo",
    "clang-repl",
    "clang-scan-deps",
    "clang-tblgen",
    "diagtool",
    "hmaptool",
    "scan-build",
    "scan-build-py",
    "scan-view",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Toolchain,
    Devel,
    ToolsExtra,
    Drop,
}

#[derive(Debug, Clone)]
pub struct TarballInfo {
    pub package: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

pub fn classify(rel: &Path) -> Bucket {
    let comps: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    if comps.is_empty() {
        return Bucket::Drop;
    }

    match comps[0] {
        "bin" | "libexec" => {
            if comps.len() < 2 {
                return Bucket::Drop;
            }
            let name = comps[1];
            let stripped = strip_clang_version(name);
            if TOOLS_EXTRA_BINS.contains(&name) || TOOLS_EXTRA_BINS.contains(&stripped.as_str()) {
                Bucket::ToolsExtra
            } else {
                // Default for unknown bin/ entries: ship in toolchain.
                let _ = TOOLCHAIN_BINS.contains(&name);
                Bucket::Toolchain
            }
        }
        "lib" => {
            // lib/clang/<v>/ → toolchain (compiler-rt builtins, sanitizer rt, builtin headers)
            if comps.len() >= 2 && comps[1] == "clang" {
                return Bucket::Toolchain;
            }
            // lib/cmake/{llvm,clang,lld} → devel
            if comps.len() >= 2 && comps[1] == "cmake" {
                return Bucket::Devel;
            }
            // lib/lib*.a → devel
            if let Some(name) = comps.last() {
                if name.starts_with("lib") && name.ends_with(".a") {
                    return Bucket::Devel;
                }
                // lib/libLLVM-<v>.{so,dylib} → toolchain (dynamic LLVM)
                if name.starts_with("libLLVM")
                    && (name.ends_with(".so") || name.ends_with(".dylib") || name.contains(".so."))
                {
                    return Bucket::Toolchain;
                }
                if name.starts_with("libclang-cpp")
                    && (name.ends_with(".so") || name.ends_with(".dylib") || name.contains(".so."))
                {
                    return Bucket::Toolchain;
                }
                if name.starts_with("libclang.") || name.starts_with("libclang-") {
                    return Bucket::Devel;
                }
                if name.starts_with("liblld") && name.ends_with(".a") {
                    return Bucket::Devel;
                }
            }
            Bucket::Toolchain
        }
        "include" => {
            // All public headers belong to devel.
            Bucket::Devel
        }
        "share" => {
            if comps.len() >= 2 && comps[1] == "clang" {
                Bucket::ToolsExtra
            } else {
                Bucket::Toolchain
            }
        }
        _ => Bucket::Toolchain,
    }
}

fn strip_clang_version(name: &str) -> String {
    // `clang-18` → `clang`, but keep `clang-tidy`/`clang-cpp` alone.
    if let Some(idx) = name.rfind('-') {
        let suffix = &name[idx + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            return name[..idx].to_string();
        }
    }
    name.to_string()
}

/// Walk `install_dir`, partition into the three packages, and write
/// each as `<output_dir>/clang-<version>-<platform>-<pkg>.tar.zst`.
/// Returns metadata for each tarball (used to populate the manifest).
pub fn package_install_tree(
    install_dir: &Path,
    output_dir: &Path,
    version: &str,
    platform: &str,
) -> Result<Vec<TarballInfo>> {
    use std::collections::BTreeMap;
    std::fs::create_dir_all(output_dir)?;

    let mut buckets: BTreeMap<&'static str, Vec<PathBuf>> = BTreeMap::new();
    buckets.insert("toolchain", vec![]);
    buckets.insert("devel", vec![]);
    buckets.insert("tools-extra", vec![]);

    for entry in WalkDir::new(install_dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(install_dir)
            .unwrap_or(entry.path());
        let bucket = classify(rel);
        let key = match bucket {
            Bucket::Toolchain => "toolchain",
            Bucket::Devel => "devel",
            Bucket::ToolsExtra => "tools-extra",
            Bucket::Drop => continue,
        };
        buckets.get_mut(key).unwrap().push(rel.to_path_buf());
    }

    let mut out = Vec::new();
    for (pkg, files) in buckets {
        if files.is_empty() {
            continue;
        }
        let tarball =
            output_dir.join(format!("clang-{version}-{platform}-{pkg}.tar.zst"));
        write_tar_zst(install_dir, &files, &tarball)
            .with_context(|| format!("pack {pkg} -> {tarball:?}"))?;
        let sha = sha256_file(&tarball)?;
        let size = std::fs::metadata(&tarball)?.len();
        out.push(TarballInfo {
            package: pkg.to_string(),
            path: tarball,
            sha256: sha,
            size,
        });
    }
    Ok(out)
}

fn write_tar_zst(install_dir: &Path, rels: &[PathBuf], dst: &Path) -> Result<()> {
    let f = File::create(dst).with_context(|| format!("create {dst:?}"))?;
    let zenc = zstd::stream::Encoder::new(BufWriter::new(f), 19)?.auto_finish();
    let mut tar = tar::Builder::new(zenc);
    tar.follow_symlinks(false);
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut sorted = rels.to_vec();
    sorted.sort();
    for rel in &sorted {
        let abs = install_dir.join(rel);
        // Add ancestor directories first so tar entries form a tree.
        let mut stack: Vec<PathBuf> = Vec::new();
        let mut cur = rel.clone();
        while let Some(parent) = cur.parent() {
            if parent.as_os_str().is_empty() {
                break;
            }
            stack.push(parent.to_path_buf());
            cur = parent.to_path_buf();
        }
        for d in stack.into_iter().rev() {
            if seen.insert(d.clone()) {
                let abs_d = install_dir.join(&d);
                if abs_d.is_dir() {
                    tar.append_dir(&d, &abs_d)?;
                }
            }
        }
        if abs.is_symlink() {
            let mut header = tar::Header::new_gnu();
            let target = std::fs::read_link(&abs)?;
            header.set_size(0);
            let meta = std::fs::symlink_metadata(&abs)?;
            header.set_metadata(&meta);
            header.set_entry_type(tar::EntryType::Symlink);
            tar.append_link(&mut header, rel, &target)?;
        } else if abs.is_file() {
            let mut f = File::open(&abs)?;
            tar.append_file(rel, &mut f)?;
        }
    }
    tar.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_paths() {
        let cases = [
            ("bin/clang", Bucket::Toolchain),
            ("bin/clang++", Bucket::Toolchain),
            ("bin/clang-18", Bucket::Toolchain),
            ("bin/clangd", Bucket::ToolsExtra),
            ("bin/clang-tidy", Bucket::ToolsExtra),
            ("bin/clang-format", Bucket::ToolsExtra),
            ("bin/lld", Bucket::Toolchain),
            ("bin/llvm-config", Bucket::Toolchain),
            ("lib/clang/18/include/stddef.h", Bucket::Toolchain),
            ("lib/libLLVM-18.dylib", Bucket::Toolchain),
            ("lib/libclang-cpp.dylib", Bucket::Toolchain),
            ("lib/libLLVMSupport.a", Bucket::Devel),
            ("lib/libclang.a", Bucket::Devel),
            ("lib/cmake/llvm/LLVMConfig.cmake", Bucket::Devel),
            ("include/llvm/ADT/StringRef.h", Bucket::Devel),
            ("include/clang/AST/Decl.h", Bucket::Devel),
            ("share/clang/clang-format-diff.py", Bucket::ToolsExtra),
        ];
        for (p, want) in cases {
            let got = classify(Path::new(p));
            assert_eq!(got, want, "classify({p}) = {got:?}, want {want:?}");
        }
    }

    #[test]
    fn package_smoke() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let install = root.join("install");
        let dist = root.join("dist");
        std::fs::create_dir_all(install.join("bin"))?;
        std::fs::create_dir_all(install.join("lib/cmake/llvm"))?;
        std::fs::create_dir_all(install.join("include/clang"))?;
        std::fs::write(install.join("bin/clang"), b"fake clang")?;
        std::fs::write(install.join("bin/clang-tidy"), b"fake tidy")?;
        std::fs::write(install.join("lib/libLLVMSupport.a"), b"fake archive")?;
        std::fs::write(install.join("lib/cmake/llvm/LLVMConfig.cmake"), b"# cfg")?;
        std::fs::write(install.join("include/clang/Foo.h"), b"// header")?;

        let outs = package_install_tree(&install, &dist, "1.2.3", "darwin-arm64")?;
        let names: Vec<&str> = outs.iter().map(|t| t.package.as_str()).collect();
        assert_eq!(names, ["devel", "toolchain", "tools-extra"]);
        for t in &outs {
            assert!(t.path.exists(), "tarball missing: {:?}", t.path);
            assert_eq!(t.sha256.len(), 64);
        }
        Ok(())
    }
}
