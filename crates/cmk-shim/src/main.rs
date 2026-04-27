// Tiny dispatcher binary symlinked into ~/.cmk/shims/<bin>.
// Reads argv[0], resolves the active version, exec's the matching
// per-version executable. Keep this fast; design §10 contemplates a
// fast-path benchmark in M8.

use std::path::Path;
use std::process::ExitCode;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("cmk-shim: {e}");
            ExitCode::from(127)
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let prog = args
        .first()
        .ok_or("missing argv[0]")?
        .clone();
    let prog_name = Path::new(&prog)
        .file_name()
        .ok_or("argv[0] has no basename")?
        .to_string_lossy()
        .to_string();

    let store = cmk_core::store::Store::open()?;
    let cwd = std::env::current_dir().ok();
    let version = cmk_core::version::resolve(&store, cwd.as_deref())?;
    let plat = cmk_core::platform::current_platform()?;
    let target = store
        .version_dir(&version, &plat)
        .join("bin")
        .join(&prog_name);

    if !target.exists() {
        return Err(format!(
            "{prog_name} not found in version {version} ({})",
            display(&target)
        )
        .into());
    }

    exec_program(&target, &args[1..])
}

#[cfg(unix)]
fn exec_program(
    target: &Path,
    rest: &[std::ffi::OsString],
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let err = std::process::Command::new(target).args(rest).exec();
    Err(format!("exec {}: {err}", display(target)).into())
}

#[cfg(not(unix))]
fn exec_program(
    target: &Path,
    rest: &[std::ffi::OsString],
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let status = std::process::Command::new(target).args(rest).status()?;
    Ok(ExitCode::from(status.code().unwrap_or(127) as u8))
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
