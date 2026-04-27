use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use url::Url;

const UA: &str = concat!("cmk-toolchain/", env!("CARGO_PKG_VERSION"));

/// Resolve a manifest-package URL into a local file path. Supports
/// `file://`, http(s), and bare filesystem paths.
pub fn fetch_to(url: &str, into_dir: &Path, fname: &str) -> Result<PathBuf> {
    fs::create_dir_all(into_dir).with_context(|| format!("mkdir {into_dir:?}"))?;
    let dst = into_dir.join(fname);

    if let Ok(parsed) = Url::parse(url) {
        match parsed.scheme() {
            "file" => {
                let src = parsed
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("bad file:// URL: {url}"))?;
                fs::copy(&src, &dst).with_context(|| format!("copy {src:?} -> {dst:?}"))?;
                return Ok(dst);
            }
            "http" | "https" => {
                fetch_http(url, &dst)?;
                return Ok(dst);
            }
            other => bail!("unsupported URL scheme `{other}`"),
        }
    }

    fs::copy(url, &dst).with_context(|| format!("copy {url} -> {dst:?}"))?;
    Ok(dst)
}

fn fetch_http(url: &str, dst: &Path) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .user_agent(UA)
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(300))
        .redirects(8)
        .build();
    let resp = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    if resp.status() / 100 != 2 {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let tmp = dst.with_extension("partial");
    let mut out = fs::File::create(&tmp).with_context(|| format!("create {tmp:?}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("read body of {url}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..n])?;
    }
    out.sync_all().ok();
    fs::rename(&tmp, dst).with_context(|| format!("rename {tmp:?} -> {dst:?}"))?;
    Ok(())
}
