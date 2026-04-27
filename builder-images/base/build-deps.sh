#!/usr/bin/env bash
# Build /opt/cmk-base/deps from source, in dep order.
# Runs INSIDE the bootstrap container, with stage0 gcc-toolset already on PATH.
# Reads versions/sha256 from /work/deps.lock (sourced as bash assoc arrays).
set -euxo pipefail

PREFIX=/opt/cmk-base/deps
SRC=/tmp/cmk-src
DL=/tmp/cmk-dl
JOBS=${JOBS:-$(nproc)}
mkdir -p "$PREFIX" "$SRC" "$DL"

# shellcheck disable=SC1091
source /work/deps.lock

fetch() { bash /work/fetch.sh "$1" "$2" "$3" "$4"; }
extract() { local f="$1" d="$2"; mkdir -p "$d"; tar -xf "$f" -C "$d" --strip-components=1; }

cd "$SRC"

# ----- zlib -----
fetch zlib "$ZLIB_URL" "$ZLIB_SHA" "$DL/zlib.tar.gz"
rm -rf zlib && mkdir zlib && extract "$DL/zlib.tar.gz" zlib
( cd zlib && ./configure --prefix="$PREFIX" --static && make -j"$JOBS" && make install )

# ----- zstd -----
fetch zstd "$ZSTD_URL" "$ZSTD_SHA" "$DL/zstd.tar.gz"
rm -rf zstd && mkdir zstd && extract "$DL/zstd.tar.gz" zstd
( cd zstd && make -j"$JOBS" PREFIX="$PREFIX" && make install PREFIX="$PREFIX" )

# ----- openssl (no docs, shared+static) -----
fetch openssl "$OPENSSL_URL" "$OPENSSL_SHA" "$DL/openssl.tar.gz"
rm -rf openssl && mkdir openssl && extract "$DL/openssl.tar.gz" openssl
( cd openssl
  ./Configure linux-aarch64 \
    --prefix="$PREFIX" --openssldir="$PREFIX/ssl" \
    no-tests no-docs shared \
    -Wl,-rpath,"$PREFIX/lib"
  make -j"$JOBS"
  make install_sw )

# ----- python (with rpath, our openssl + zlib) -----
fetch python "$PYTHON_URL" "$PYTHON_SHA" "$DL/python.tar.xz"
rm -rf python && mkdir python && extract "$DL/python.tar.xz" python
( cd python
  ./configure \
    --prefix="$PREFIX" \
    --enable-shared \
    --with-openssl="$PREFIX" \
    --with-system-ffi \
    --enable-optimizations=no \
    LDFLAGS="-Wl,-rpath,$PREFIX/lib -L$PREFIX/lib" \
    CPPFLAGS="-I$PREFIX/include"
  make -j"$JOBS"
  make install
  ln -sf python3 "$PREFIX/bin/python" )

# ----- cmake (use --system-* off; vendored OpenSSL/zlib via our PREFIX) -----
fetch cmake "$CMAKE_URL" "$CMAKE_SHA" "$DL/cmake.tar.gz"
rm -rf cmake && mkdir cmake && extract "$DL/cmake.tar.gz" cmake
( cd cmake
  ./bootstrap --prefix="$PREFIX" --parallel="$JOBS" \
    -- -DCMAKE_USE_OPENSSL=ON -DOPENSSL_ROOT_DIR="$PREFIX"
  make -j"$JOBS"
  make install )

# ----- ninja -----
fetch ninja "$NINJA_URL" "$NINJA_SHA" "$DL/ninja.tar.gz"
rm -rf ninja && mkdir ninja && extract "$DL/ninja.tar.gz" ninja
( cd ninja
  "$PREFIX/bin/cmake" -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX"
  "$PREFIX/bin/cmake" --build build -j"$JOBS"
  "$PREFIX/bin/cmake" --install build )

# ----- git (no gettext, no perl docs) -----
fetch git "$GIT_URL" "$GIT_SHA" "$DL/git.tar.xz"
rm -rf git && mkdir git && extract "$DL/git.tar.xz" git
( cd git
  make configure
  ./configure --prefix="$PREFIX" --without-tcltk \
    CFLAGS="-I$PREFIX/include" LDFLAGS="-L$PREFIX/lib -Wl,-rpath,$PREFIX/lib" \
    LIBS="-lz"
  make -j"$JOBS" NO_GETTEXT=1 NO_PERL=1 NO_TCLTK=1 NO_INSTALL_HARDLINKS=1 all
  make NO_GETTEXT=1 NO_PERL=1 NO_TCLTK=1 NO_INSTALL_HARDLINKS=1 install )

# ----- ccache -----
fetch ccache "$CCACHE_URL" "$CCACHE_SHA" "$DL/ccache.tar.xz"
rm -rf ccache && mkdir ccache && extract "$DL/ccache.tar.xz" ccache
( cd ccache
  "$PREFIX/bin/cmake" -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DREDIS_STORAGE_BACKEND=OFF -DENABLE_TESTING=OFF \
    -DZSTD_FROM_INTERNET=OFF -DZSTD_LIBRARY="$PREFIX/lib/libzstd.so" \
    -DZSTD_INCLUDE_DIR="$PREFIX/include"
  "$PREFIX/bin/cmake" --build build -j"$JOBS"
  "$PREFIX/bin/cmake" --install build )

echo "==== /opt/cmk-base/deps populated ===="
ls "$PREFIX/bin"
