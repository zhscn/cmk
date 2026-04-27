use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::cmake::{self, cmake_configure, ninja, ninja_install};
use crate::package::{TarballInfo, package_install_tree};
use crate::provenance::{PlatformDescriptor, Provenance, write_manifest_fragment};
use crate::source::{self, SourceSpec};

pub struct MacosBuildArgs {
    pub version: String,
    pub source: SourceSpec,
    pub work_dir: PathBuf,
    pub output_dir: PathBuf,
    pub downloads: PathBuf,
    pub jobs: usize,
}

#[derive(Debug, Clone)]
pub struct BuildOutputs {
    pub install_dir: PathBuf,
    pub tarballs: Vec<TarballInfo>,
    pub manifest_fragment: PathBuf,
}

/// Drive the macOS single-stage build:
///   1. host self-check (system clang must compile a C/C++ probe)
///   2. final cmake (clang+lld+clang-tools-extra+compiler-rt builtins/CRT)
///   3. compiler-rt pass2 with the just-built clang (sanitizers)
///   4. package toolchain/devel/tools-extra into tar.zst
pub fn run(args: &MacosBuildArgs) -> Result<BuildOutputs> {
    eprintln!("cmk toolchain builder: macOS arm64, version {}", args.version);

    let llvm_src = args.work_dir.join("llvm-src");
    let final_build = args.work_dir.join("final-build");
    let install_dir = args.work_dir.join("install");
    let rt_build = args.work_dir.join("rt-build");

    let mut provenance = Provenance::current();
    if !llvm_src.join("llvm").is_dir() {
        provenance.source = source::prepare(&args.source, &llvm_src, &args.downloads)?;
    } else {
        eprintln!("source: reusing {llvm_src:?}");
    }
    host_self_check()?;

    let install_str = install_dir.to_string_lossy().to_string();
    let mut cfg: Vec<String> = vec![
        "-DCMAKE_BUILD_TYPE=Release".into(),
        format!("-DCMAKE_INSTALL_PREFIX={install_str}"),
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=13.0".into(),
        "-DLLVM_ENABLE_PROJECTS=clang;lld;clang-tools-extra".into(),
        "-DLLVM_ENABLE_RUNTIMES=compiler-rt".into(),
        "-DLLVM_TARGETS_TO_BUILD=AArch64".into(),
        "-DLLVM_ENABLE_LIBXML2=OFF".into(),
        "-DLLVM_ENABLE_ZLIB=ON".into(),
        "-DLLVM_ENABLE_ASSERTIONS=OFF".into(),
        "-DLLVM_INCLUDE_TESTS=OFF".into(),
        "-DLLVM_INCLUDE_EXAMPLES=OFF".into(),
        "-DLLVM_INCLUDE_BENCHMARKS=OFF".into(),
        "-DLLVM_BUILD_LLVM_DYLIB=ON".into(),
        "-DLLVM_LINK_LLVM_DYLIB=ON".into(),
        "-DCLANG_LINK_CLANG_DYLIB=ON".into(),
        "-DCOMPILER_RT_BUILD_BUILTINS=ON".into(),
        "-DCOMPILER_RT_BUILD_CRT=ON".into(),
        "-DCOMPILER_RT_BUILD_SANITIZERS=OFF".into(),
        "-DCOMPILER_RT_INCLUDE_TESTS=OFF".into(),
        "-DCMAKE_INSTALL_RPATH=@loader_path/../lib".into(),
        "-DCMAKE_BUILD_WITH_INSTALL_RPATH=ON".into(),
    ];
    if let Some(sdk) = xcode_sdk() {
        cfg.push(format!("-DCMAKE_OSX_SYSROOT={sdk}"));
    }

    cmake_configure(&final_build, &llvm_src.join("llvm"), &cfg, &[])?;
    ninja(&final_build, None, args.jobs)?;
    ninja_install(&final_build)?;

    // Pass 2: compiler-rt sanitizers using the just-built clang. The
    // compiler-rt source lives in the monorepo under `compiler-rt/`.
    let clang = install_dir.join("bin/clang");
    let clangxx = install_dir.join("bin/clang++");
    let llvm_config = install_dir.join("bin/llvm-config");
    anyhow::ensure!(clang.exists(), "missing {clang:?} after final stage");

    let triple = "arm64-apple-darwin";
    let rt_args: Vec<String> = vec![
        "-DCMAKE_BUILD_TYPE=Release".into(),
        format!("-DCMAKE_INSTALL_PREFIX={install_str}"),
        format!("-DCMAKE_C_COMPILER={}", clang.display()),
        format!("-DCMAKE_CXX_COMPILER={}", clangxx.display()),
        format!("-DCMAKE_C_COMPILER_TARGET={triple}"),
        format!("-DCMAKE_CXX_COMPILER_TARGET={triple}"),
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=13.0".into(),
        "-DCOMPILER_RT_BUILD_BUILTINS=OFF".into(),
        "-DCOMPILER_RT_BUILD_CRT=OFF".into(),
        "-DCOMPILER_RT_BUILD_SANITIZERS=ON".into(),
        "-DCOMPILER_RT_INCLUDE_TESTS=OFF".into(),
        format!("-DLLVM_CONFIG_PATH={}", llvm_config.display()),
    ];
    let rt_src = llvm_src.join("compiler-rt");
    anyhow::ensure!(
        rt_src.is_dir(),
        "compiler-rt source not found at {rt_src:?}"
    );
    cmake_configure(&rt_build, &rt_src, &rt_args, &[])?;
    ninja(&rt_build, None, args.jobs)?;
    ninja_install(&rt_build)?;

    let tarballs =
        package_install_tree(&install_dir, &args.output_dir, &args.version, "darwin-arm64")?;

    let plat = PlatformDescriptor {
        key: "darwin-arm64".into(),
        baseline: "macos-13".into(),
        host_glibc_min: None,
        system_libcxx: true,
        system_unwinder: true,
    };
    let manifest_fragment =
        write_manifest_fragment(&args.output_dir, &args.version, &plat, &tarballs, &provenance)?;

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

fn host_self_check() -> Result<()> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "clang".into());
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "clang++".into());

    let tmp = std::env::temp_dir().join(format!("cmk-host-check-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let cpp = tmp.join("t.cpp");
    std::fs::write(&cpp, b"#include <iostream>\nint main(){std::cout<<\"ok\\n\";}\n")?;
    let bin = tmp.join("t");
    let status = Command::new(&cxx)
        .arg("-std=c++17")
        .arg(&cpp)
        .arg("-o")
        .arg(&bin)
        .status()
        .with_context(|| format!("run host {cxx}"))?;
    if !status.success() {
        bail!("host self-check failed (compile)");
    }
    let out = Command::new(&bin).output()?;
    if !out.status.success() || out.stdout != b"ok\n" {
        bail!("host self-check failed (run)");
    }
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("host self-check: OK ({cc}, {cxx})");
    Ok(())
}

fn xcode_sdk() -> Option<String> {
    let out = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub use cmake::detect_jobs;
