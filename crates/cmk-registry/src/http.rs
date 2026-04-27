use std::time::Duration;

use anyhow::{Context, Result, bail};

const UA: &str = concat!("cmk-registry/", env!("CARGO_PKG_VERSION"));

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?)
}

pub async fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = client()?
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }
    // 64 MiB cap is enforced by Content-Length when the server provides it;
    // we read the full body otherwise (manifest/index payloads are small).
    if let Some(len) = resp.content_length()
        && len > 64 * 1024 * 1024
    {
        bail!("body of {url} too large: {len} bytes");
    }
    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("read body of {url}"))?;
    Ok(bytes.to_vec())
}

pub async fn get_string(url: &str) -> Result<String> {
    let bytes = get_bytes(url).await?;
    String::from_utf8(bytes).with_context(|| format!("non-utf8 body from {url}"))
}
