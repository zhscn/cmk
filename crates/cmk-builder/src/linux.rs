use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::cmake::run as run_cmd;
use crate::container::ContainerRuntime;
use crate::macos::BuildOutputs;
use crate::package::package_install_tree;
use crate::provenance::{PlatformDescriptor, Provenance, write_manifest_fragment};
use crate::recipe::{Arch, Target};
use crate::runtime;
use crate::source::{self, SourceSpec};

pub struct LinuxBuildArgs {
    pub version: String,
    pub target: Target,
    pub source: SourceSpec,
    pub work_dir: PathBuf,
    pub output_dir: PathBuf,
    pub downloads: PathBuf,
    pub image: String,
    pub runtime: Option<ContainerRuntime>,
    pub jobs: usize,
    pub ccache_dir: Option<PathBuf>,
    pub shell: bool,
}

/// Drive the Linux build pipeline:
///   1. fetch + extract llvm-project on host (network is on host only)
///   2. write `run-build.sh` into work_dir
///   3. invoke docker/podman with `--network=none --read-only` and a
///      single rw mount of work_dir, using
///      `/opt/cmk-base/bootstrap-clang` from the base image as the
///      host toolchain (no separate bootstrap stage needed)
///   4. on host: walk work_dir/install and pack tarballs per design §3.1
pub fn run(args: &LinuxBuildArgs) -> Result<BuildOutputs> {
    let plat = args.target.platform_key();
    eprintln!("cmk toolchain builder: {plat}, version {}", args.version);

    let llvm_src = args.work_dir.join("llvm-src");
    let install_dir = args.work_dir.join("install");
    let build_dir = args.work_dir.join("build");
    let rt_build = args.work_dir.join("rt-build");
    for d in [&llvm_src, &install_dir, &build_dir, &rt_build] {
        std::fs::create_dir_all(d)?;
    }

    let mut provenance = Provenance::current();
    provenance.builder_base = Some(args.image.clone());
    if !llvm_src.join("llvm").is_dir() {
        provenance.source = source::prepare(&args.source, &llvm_src, &args.downloads)?;
    } else {
        eprintln!("source: reusing {llvm_src:?}");
    }

    let script_path = args.work_dir.join("run-build.sh");
    std::fs::write(&script_path, render_script(args.target, args.jobs))
        .with_context(|| format!("write {script_path:?}"))?;
    set_exec(&script_path)?;

    let (rt, bin) = match args.runtime {
        Some(rt) => (
            rt,
            runtime::locate(rt).ok_or_else(|| anyhow::anyhow!("{rt:?} not in PATH"))?,
        ),
        None => runtime::detect().ok_or_else(|| {
            anyhow::anyhow!("docker/podman not found; install one or pass --runtime")
        })?,
    };
    eprintln!("container runtime: {rt:?} ({})", bin.display());

    invoke_container(&bin, rt, args, &script_path)?;

    let tarballs = package_install_tree(&install_dir, &args.output_dir, &args.version, plat)?;

    let baseline = match args.target.arch {
        Arch::X86_64 => "el7",
        Arch::Aarch64 => "el8",
    };
    let glibc = match args.target.arch {
        Arch::X86_64 => Some("2.17".into()),
        Arch::Aarch64 => Some("2.28".into()),
    };
    let plat_desc = PlatformDescriptor {
        key: plat.into(),
        baseline: baseline.into(),
        host_glibc_min: glibc,
        system_libcxx: false,
        system_unwinder: false,
    };
    let manifest_fragment = write_manifest_fragment(
        &args.output_dir,
        &args.version,
        &plat_desc,
        &tarballs,
        &provenance,
    )?;

    eprintln!("cmk toolchain builder: produced {} tarballs", tarballs.len());
    for t in &tarballs {
        eprintln!("  {} {} ({} bytes)", t.package, t.sha256, t.size);
    }
    eprintln!("manifest fragment: {}", manifest_fragment.display());

    Ok(BuildOutputs {
        install_dir,
        tarballs,
        manifest_fragment,
    })
}

fn invoke_container(
    bin: &Path,
    _rt: ContainerRuntime,
    args: &LinuxBuildArgs,
    script_in_work: &Path,
) -> Result<()> {
    let (uid, gid) = runtime::current_uid_gid();
    let mut cmd = Command::new(bin);
    cmd.arg("run").arg("--rm");
    if !args.shell {
        // Interactive shell (`--shell`) wants stdin attached; the
        // batch case is non-interactive.
        cmd.arg("-i");
    } else {
        cmd.arg("-it");
    }
    cmd.arg("--network=none")
        .arg("--read-only")
        .arg("--tmpfs")
        .arg("/tmp:rw,exec,size=4g")
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .arg("-v")
        .arg(format!("{}:/work:rw", canonical(&args.work_dir)?))
        .arg("-e")
        .arg(format!("JOBS={}", args.jobs))
        .arg("-e")
        .arg("HOME=/tmp/cmk-home");

    if let Some(c) = &args.ccache_dir {
        std::fs::create_dir_all(c).ok();
        cmd.arg("-v")
            .arg(format!("{}:/ccache:rw", canonical(c)?))
            .arg("-e")
            .arg("CCACHE_DIR=/ccache");
    }

    cmd.arg("-w").arg("/work").arg(&args.image);
    let script_in_container = format!(
        "/work/{}",
        script_in_work
            .file_name()
            .unwrap()
            .to_string_lossy()
    );
    if args.shell {
        cmd.arg("/bin/bash");
    } else {
        cmd.arg("/bin/bash").arg(&script_in_container);
    }
    run_cmd(&mut cmd, "container")
}

fn canonical(p: &Path) -> Result<String> {
    let abs = std::fs::canonicalize(p).with_context(|| format!("canonicalize {p:?}"))?;
    Ok(abs.to_string_lossy().to_string())
}

#[cfg(unix)]
fn set_exec(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p)?.permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(p, perm)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_exec(_p: &Path) -> Result<()> {
    Ok(())
}

fn render_script(target: Target, _jobs: usize) -> String {
    let llvm_target = target.llvm_target();
    let triple = target.triple();

    // The base image entrypoint sees /opt/cmk-base/bootstrap-clang
    // and /opt/cmk-base/deps already on PATH (set in the Dockerfile),
    // but we re-export inside the script for `--shell` debugging too.
    format!(
        r#"#!/bin/bash
set -euxo pipefail

JOBS=${{JOBS:-$(nproc)}}
WORK=/work
LLVM_SRC=$WORK/llvm-src
BUILD=$WORK/build
INSTALL=$WORK/install
RT_BUILD=$WORK/rt-build
mkdir -p $BUILD $INSTALL $RT_BUILD

export PATH=/opt/cmk-base/bootstrap-clang/bin:/opt/cmk-base/deps/bin:$PATH

CMAKE=/opt/cmk-base/deps/bin/cmake
NINJA=/opt/cmk-base/deps/bin/ninja
PYTHON=/opt/cmk-base/deps/bin/python3
HOST_CC=/opt/cmk-base/bootstrap-clang/bin/clang
HOST_CXX=/opt/cmk-base/bootstrap-clang/bin/clang++

LAUNCHER_FLAGS=()
if command -v ccache >/dev/null 2>&1; then
  LAUNCHER_FLAGS+=( -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache )
fi

$CMAKE -G Ninja "$LLVM_SRC/llvm" \
  -B "$BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$INSTALL" \
  -DCMAKE_C_COMPILER="$HOST_CC" \
  -DCMAKE_CXX_COMPILER="$HOST_CXX" \
  -DLLVM_USE_LINKER=lld \
  -DLLVM_ENABLE_PROJECTS="clang;lld;clang-tools-extra" \
  -DLLVM_ENABLE_RUNTIMES="compiler-rt;libcxx;libcxxabi" \
  -DLLVM_TARGETS_TO_BUILD="{llvm_target}" \
  -DLLVM_ENABLE_LIBXML2=OFF \
  -DLLVM_ENABLE_ZLIB=ON \
  -DZLIB_ROOT=/opt/cmk-base/deps \
  -DLLVM_ENABLE_ASSERTIONS=OFF \
  -DLLVM_INCLUDE_TESTS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF \
  -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_BUILD_LLVM_DYLIB=ON \
  -DLLVM_LINK_LLVM_DYLIB=ON \
  -DCLANG_LINK_CLANG_DYLIB=ON \
  -DLLVM_STATIC_LINK_CXX_STDLIB=ON \
  -DCLANG_DEFAULT_CXX_STDLIB=libc++ \
  -DCLANG_DEFAULT_RTLIB=compiler-rt \
  -DCLANG_DEFAULT_UNWINDLIB=libgcc \
  -DCLANG_DEFAULT_LINKER=lld \
  -DCOMPILER_RT_BUILD_BUILTINS=ON \
  -DCOMPILER_RT_BUILD_CRT=ON \
  -DCOMPILER_RT_BUILD_SANITIZERS=OFF \
  -DLIBCXX_ENABLE_SHARED=OFF \
  -DLIBCXX_ENABLE_STATIC=ON \
  -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON \
  -DLIBCXXABI_ENABLE_SHARED=OFF \
  -DLIBCXXABI_ENABLE_STATIC=ON \
  -DLIBCXXABI_USE_LLVM_UNWINDER=OFF \
  -DCMAKE_INSTALL_RPATH='$ORIGIN/../lib' \
  -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
  -DPython3_EXECUTABLE="$PYTHON" \
  "${{LAUNCHER_FLAGS[@]}}"

$NINJA -C "$BUILD" -j "$JOBS"
$NINJA -C "$BUILD" install

$CMAKE -G Ninja "$LLVM_SRC/compiler-rt" \
  -B "$RT_BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$INSTALL" \
  -DCMAKE_C_COMPILER="$INSTALL/bin/clang" \
  -DCMAKE_CXX_COMPILER="$INSTALL/bin/clang++" \
  -DCMAKE_C_COMPILER_TARGET="{triple}" \
  -DCMAKE_CXX_COMPILER_TARGET="{triple}" \
  -DCOMPILER_RT_BUILD_BUILTINS=OFF \
  -DCOMPILER_RT_BUILD_CRT=OFF \
  -DCOMPILER_RT_BUILD_SANITIZERS=ON \
  -DCOMPILER_RT_INCLUDE_TESTS=OFF \
  -DLLVM_CONFIG_PATH="$INSTALL/bin/llvm-config"

$NINJA -C "$RT_BUILD" -j "$JOBS"
$NINJA -C "$RT_BUILD" install
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{Arch, Os};

    #[test]
    fn x86_script_has_design_flags() {
        let t = Target {
            os: Os::Linux,
            arch: Arch::X86_64,
        };
        let s = render_script(t, 8);
        for needle in [
            "LLVM_TARGETS_TO_BUILD=\"X86\"",
            "LLVM_ENABLE_PROJECTS=\"clang;lld;clang-tools-extra\"",
            "LLVM_ENABLE_RUNTIMES=\"compiler-rt;libcxx;libcxxabi\"",
            "LLVM_USE_LINKER=lld",
            "LLVM_BUILD_LLVM_DYLIB=ON",
            "LLVM_LINK_LLVM_DYLIB=ON",
            "CLANG_LINK_CLANG_DYLIB=ON",
            "CLANG_DEFAULT_CXX_STDLIB=libc++",
            "CLANG_DEFAULT_RTLIB=compiler-rt",
            "CLANG_DEFAULT_UNWINDLIB=libgcc",
            "CLANG_DEFAULT_LINKER=lld",
            "LIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON",
            "LIBCXXABI_USE_LLVM_UNWINDER=OFF",
            "/opt/cmk-base/bootstrap-clang/bin/clang",
            "x86_64-unknown-linux-gnu",
            "COMPILER_RT_BUILD_SANITIZERS=ON",
        ] {
            assert!(!s.contains("LLVM_ENABLE_LLD=ON"), "ENABLE_LLD must not coexist with USE_LINKER=lld");
            assert!(s.contains(needle), "script missing `{needle}`:\n{s}");
        }
    }

    #[test]
    fn aarch64_script_uses_aarch64_target() {
        let t = Target {
            os: Os::Linux,
            arch: Arch::Aarch64,
        };
        let s = render_script(t, 8);
        assert!(s.contains("LLVM_TARGETS_TO_BUILD=\"AArch64\""));
        assert!(s.contains("aarch64-unknown-linux-gnu"));
    }
}
