use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const UA: &str = concat!("cmk-toolchain/", env!("CARGO_PKG_VERSION"));

pub fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .user_agent(UA)
        .timeout(Duration::from_secs(60))
        .redirects(8)
        .build();
    let resp = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    if resp.status() / 100 != 2 {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let mut buf = Vec::new();
    resp.into_reader()
        .take(64 * 1024 * 1024) // 64 MiB cap for index/manifest payloads
        .read_to_end(&mut buf)
        .with_context(|| format!("read body of {url}"))?;
    Ok(buf)
}

pub fn get_string(url: &str) -> Result<String> {
    let bytes = get_bytes(url)?;
    String::from_utf8(bytes).with_context(|| format!("non-utf8 body from {url}"))
}
