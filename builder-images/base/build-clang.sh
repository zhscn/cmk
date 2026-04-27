#!/usr/bin/env bash
# Two-stage clang bootstrap into /opt/cmk-base/bootstrap-clang.
# Stage1: stage0 gcc + libstdc++  ->  minimal clang  (in /tmp/stage1)
# Stage2: stage1 clang + libc++/libc++abi/compiler-rt  ->  /opt/cmk-base/bootstrap-clang
#
# Inputs from env:
#   - LLVM source already fetched + extracted at $LLVM_SRC
#   - $PREFIX = /opt/cmk-base/deps  (cmake/ninja/python/zlib live here)
#   - JOBS
set -euxo pipefail

PREFIX=/opt/cmk-base/deps
FINAL=/opt/cmk-base/bootstrap-clang
LLVM_SRC=${LLVM_SRC:-/tmp/cmk-src/llvm-project}
JOBS=${JOBS:-$(nproc)}

CMAKE="$PREFIX/bin/cmake"
NINJA="$PREFIX/bin/ninja"
PYTHON="$PREFIX/bin/python3"

# stage0 gcc-toolset isn't always on PATH in fresh RUN layers — locate it once
# and pass explicitly to cmake (avoids relying on Dockerfile ENV which would
# otherwise invalidate the deps cache when changed).
GCC_BIN=/opt/rh/gcc-toolset-12/root/usr/bin
GCC0="$GCC_BIN/gcc"
GXX0="$GCC_BIN/g++"
export PATH="$PREFIX/bin:$GCC_BIN:$PATH"

# Detect arch -> LLVM target
ARCH=$(uname -m)
case "$ARCH" in
  aarch64|arm64) LLVM_TARGET=AArch64 ; TRIPLE=aarch64-unknown-linux-gnu ;;
  x86_64)        LLVM_TARGET=X86     ; TRIPLE=x86_64-unknown-linux-gnu ;;
  *) echo "unsupported arch $ARCH" >&2; exit 1 ;;
esac

STAGE1=/tmp/stage1
STAGE1_BUILD=/tmp/stage1-build
mkdir -p "$STAGE1" "$STAGE1_BUILD"

# ---------- stage1: stage0 gcc -> stripped-down clang+lld ----------
"$CMAKE" -G Ninja "$LLVM_SRC/llvm" \
  -B "$STAGE1_BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$STAGE1" \
  -DCMAKE_MAKE_PROGRAM="$NINJA" \
  -DCMAKE_C_COMPILER="$GCC0" \
  -DCMAKE_CXX_COMPILER="$GXX0" \
  -DLLVM_ENABLE_PROJECTS="clang;lld" \
  -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGET" \
  -DLLVM_ENABLE_LIBXML2=OFF \
  -DLLVM_ENABLE_ZLIB=ON -DZLIB_ROOT="$PREFIX" \
  -DLLVM_ENABLE_ZSTD=OFF \
  -DLLVM_ENABLE_TERMINFO=OFF \
  -DLLVM_INCLUDE_TESTS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF \
  -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_DOCS=OFF \
  -DLLVM_ENABLE_ASSERTIONS=OFF \
  -DPython3_EXECUTABLE="$PYTHON"

"$NINJA" -C "$STAGE1_BUILD" -j"$JOBS" install-clang install-clang-resource-headers \
  install-lld install-llvm-ar install-llvm-ranlib install-llvm-nm install-llvm-strip \
  install-llvm-objdump install-llvm-objcopy install-llvm-readelf install-llvm-symbolizer \
  install-LTO install-llvm-config

# Free the stage1 build dir before stage2 (saves ~10GB)
rm -rf "$STAGE1_BUILD"

# ---------- stage2: stage1 clang -> final bootstrap-clang with libc++ ----------
STAGE2_BUILD=/tmp/stage2-build
mkdir -p "$FINAL" "$STAGE2_BUILD"

"$CMAKE" -G Ninja "$LLVM_SRC/llvm" \
  -B "$STAGE2_BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$FINAL" \
  -DCMAKE_MAKE_PROGRAM="$NINJA" \
  -DCMAKE_C_COMPILER="$STAGE1/bin/clang" \
  -DCMAKE_CXX_COMPILER="$STAGE1/bin/clang++" \
  -DLLVM_USE_LINKER=lld \
  -DLLVM_ENABLE_PROJECTS="clang;lld" \
  -DLLVM_ENABLE_RUNTIMES="compiler-rt;libcxx;libcxxabi" \
  -DLLVM_TARGETS_TO_BUILD="$LLVM_TARGET" \
  -DLLVM_ENABLE_LIBXML2=OFF \
  -DLLVM_ENABLE_ZLIB=ON -DZLIB_ROOT="$PREFIX" \
  -DLLVM_ENABLE_ZSTD=ON -DLLVM_USE_STATIC_ZSTD=ON \
  -DZSTD_INCLUDE_DIR="$PREFIX/include" -DZSTD_LIBRARY="$PREFIX/lib/libzstd.a" \
  -DLLVM_ENABLE_TERMINFO=OFF \
  -DLLVM_INCLUDE_TESTS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF \
  -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_DOCS=OFF \
  -DLLVM_ENABLE_ASSERTIONS=OFF \
  -DLLVM_BUILD_LLVM_DYLIB=ON -DLLVM_LINK_LLVM_DYLIB=ON \
  -DCLANG_LINK_CLANG_DYLIB=ON \
  -DLLVM_STATIC_LINK_CXX_STDLIB=ON \
  -DCLANG_DEFAULT_CXX_STDLIB=libc++ \
  -DCLANG_DEFAULT_RTLIB=compiler-rt \
  -DCLANG_DEFAULT_UNWINDLIB=libgcc \
  -DCLANG_DEFAULT_LINKER=lld \
  -DCOMPILER_RT_BUILD_BUILTINS=ON \
  -DCOMPILER_RT_BUILD_CRT=ON \
  -DCOMPILER_RT_BUILD_SANITIZERS=OFF \
  -DCOMPILER_RT_BUILD_LIBFUZZER=OFF \
  -DCOMPILER_RT_BUILD_MEMPROF=OFF \
  -DCOMPILER_RT_BUILD_PROFILE=OFF \
  -DCOMPILER_RT_BUILD_XRAY=OFF \
  -DCOMPILER_RT_BUILD_ORC=OFF \
  -DLIBCXX_ENABLE_SHARED=OFF -DLIBCXX_ENABLE_STATIC=ON \
  -DLIBCXX_ENABLE_STATIC_ABI_LIBRARY=ON \
  -DLIBCXXABI_ENABLE_SHARED=OFF -DLIBCXXABI_ENABLE_STATIC=ON \
  -DLIBCXXABI_USE_LLVM_UNWINDER=OFF \
  -DCMAKE_INSTALL_RPATH='$ORIGIN/../lib' \
  -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
  -DPython3_EXECUTABLE="$PYTHON"

"$NINJA" -C "$STAGE2_BUILD" -j"$JOBS"
"$NINJA" -C "$STAGE2_BUILD" install

rm -rf "$STAGE2_BUILD" "$STAGE1"

echo "==== bootstrap-clang at $FINAL ===="
"$FINAL/bin/clang" --version
