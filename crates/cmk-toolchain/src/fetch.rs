use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use url::Url;

const UA: &str = concat!("cmk-toolchain/", env!("CARGO_PKG_VERSION"));

/// Sync entry point for callers running inside `spawn_blocking` or other
/// non-async contexts (e.g. the builder). Spins up a current-thread tokio
/// runtime to drive `fetch_to`.
pub fn fetch_to_blocking(url: &str, into_dir: &Path, fname: &str) -> Result<PathBuf> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .with_context(|| "build tokio runtime for blocking fetch")?;
    rt.block_on(fetch_to(url, into_dir, fname))
}

/// Resolve a manifest-package URL into a local file path. Supports
/// `file://`, http(s), and bare filesystem paths.
pub async fn fetch_to(url: &str, into_dir: &Path, fname: &str) -> Result<PathBuf> {
    fs::create_dir_all(into_dir)
        .await
        .with_context(|| format!("mkdir {into_dir:?}"))?;
    let dst = into_dir.join(fname);

    if let Ok(parsed) = Url::parse(url) {
        match parsed.scheme() {
            "file" => {
                let src = parsed
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("bad file:// URL: {url}"))?;
                fs::copy(&src, &dst)
                    .await
                    .with_context(|| format!("copy {src:?} -> {dst:?}"))?;
                return Ok(dst);
            }
            "http" | "https" => {
                fetch_http(url, &dst).await?;
                return Ok(dst);
            }
            other => bail!("unsupported URL scheme `{other}`"),
        }
    }

    fs::copy(url, &dst)
        .await
        .with_context(|| format!("copy {url} -> {dst:?}"))?;
    Ok(dst)
}

async fn fetch_http(url: &str, dst: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .with_context(|| "build reqwest client")?;

    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }

    let tmp = dst.with_extension("partial");
    let mut out = fs::File::create(&tmp)
        .await
        .with_context(|| format!("create {tmp:?}"))?;
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("read body of {url}"))?;
        out.write_all(&chunk).await?;
    }
    out.sync_all().await.ok();
    fs::rename(&tmp, dst)
        .await
        .with_context(|| format!("rename {tmp:?} -> {dst:?}"))?;
    Ok(())
}
