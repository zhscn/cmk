use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Compute sha256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = BufReader::new(File::open(path).with_context(|| format!("open {path:?}"))?);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn verify_sha256(path: &Path, want: &str) -> Result<()> {
    let got = sha256_file(path)?;
    if !eq_ignore_ascii_case(&got, want) {
        bail!("sha256 mismatch for {path:?}: got {got}, want {want}");
    }
    Ok(())
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

/// Extract a (zstd|gz|xz|plain) tar archive into `dest`. Returns the
/// list of paths (relative to `dest`) that were unpacked, used to
/// record file ownership in installed.json so `cmk toolchain remove` knows
/// what to delete.
pub fn extract_tar_auto(archive: &Path, dest: &Path) -> Result<Vec<String>> {
    std::fs::create_dir_all(dest)?;
    let f = File::open(archive).with_context(|| format!("open {archive:?}"))?;
    let reader: Box<dyn Read> = match detect(archive)? {
        Compression::Zstd => Box::new(zstd::stream::Decoder::new(f)?),
        Compression::Gzip => Box::new(flate2::read::GzDecoder::new(f)),
        Compression::Xz => Box::new(xz2::read::XzDecoder::new(f)),
        Compression::None => Box::new(BufReader::new(f)),
    };
    let mut tar = tar::Archive::new(reader);
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);

    let mut files = Vec::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        entry.unpack_in(dest)?;
        files.push(path_to_string(&path));
    }
    Ok(files)
}

#[derive(Copy, Clone, Debug)]
enum Compression {
    Zstd,
    Gzip,
    Xz,
    None,
}

fn detect(path: &Path) -> io::Result<Compression> {
    let mut f = File::open(path)?;
    let mut head = [0u8; 6];
    let n = f.read(&mut head)?;
    let head = &head[..n];
    Ok(match head {
        [0x28, 0xB5, 0x2F, 0xFD, ..] => Compression::Zstd,
        [0x1F, 0x8B, ..] => Compression::Gzip,
        [0xFD, b'7', b'z', b'X', b'Z', 0x00, ..] => Compression::Xz,
        _ => Compression::None,
    })
}

fn path_to_string(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Convenience: walk an installed prefix and remove the recorded files
/// (used by `cmk toolchain remove`). Empty parent directories that became
/// empty are pruned bottom-up.
pub fn prune_files(prefix: &Path, files: &[String]) -> Result<()> {
    let mut paths: Vec<PathBuf> = files.iter().map(|f| prefix.join(f)).collect();
    // Longest paths first so files come before directories.
    paths.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for p in &paths {
        if p.is_symlink() || p.is_file() {
            let _ = std::fs::remove_file(p);
        } else if p.is_dir() {
            let _ = std::fs::remove_dir(p);
        }
    }
    Ok(())
}
