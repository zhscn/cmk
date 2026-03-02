use anyhow::Result;
use std::{cmp::min, process::Stdio};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(crate) async fn wait_with_cancel(
    child: &mut tokio::process::Child,
) -> Result<std::process::ExitStatus> {
    tokio::select! {
        status = child.wait() => Ok(status?),
        _ = tokio::signal::ctrl_c() => {
            child.kill().await.ok();
            child.wait().await.ok();
            std::process::exit(130)
        }
    }
}

pub async fn completing_read(elements: &[String]) -> Result<String> {
    let height = min(elements.len(), 10) + 2;
    let mut fzf = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(["--height", &height.to_string()])
        .spawn()?;
    let mut child_stdin = fzf.stdin.take().unwrap();
    for element in elements {
        child_stdin.write_all(element.as_bytes()).await?;
        child_stdin.write_all(b"\n").await?;
    }
    drop(child_stdin);
    let output = fzf.wait_with_output().await?;
    if !output.status.success() {
        // fzf exits 130 on Ctrl+C, 1 on Esc/no-match — treat all as user cancellation
        std::process::exit(130);
    }
    let mut stdout = output.stdout;
    if stdout.ends_with(b"\n") {
        stdout.pop();
    }
    Ok(String::from_utf8(stdout)?)
}
