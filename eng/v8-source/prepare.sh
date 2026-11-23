#!/usr/bin/env bash
# Acquire and verify the exact inputs for a from-source v8 (rusty_v8) build.
#
# The crates.io archive `v8` 150.4.0 deliberately omits four inputs that a
# source build needs: the GN binary, the Chromium Rust toolchain, the
# `third_party/rust` vendor tree, and the ICU data blobs. Each has an exact
# revision pinned by the crate's own DEPS files; this script fetches those
# revisions and checks the hash or commit before anything is used.
#
# It never writes to the cargo registry, never reads a credential, and is
# idempotent: an already-verified input is left untouched. Run with
# `--verify-only` to check an existing tree without downloading or writing.
#
# Host tools are not provisioned here. The build also needs: python3, ninja,
# curl, git, and a C++ compiler able to build GN (clang++ or g++).

set -euo pipefail

CACHE=${NEOTALES_V8_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-v8-build}

VERIFY_ONLY=0
case ${1:-} in
    --verify-only) VERIFY_ONLY=1 ;;
    "") ;;
    *)
        printf 'usage: %s [--verify-only]\n' "$0" >&2
        exit 2
        ;;
esac

# --- Pins ------------------------------------------------------------------
# The crate archive is the root of trust for the V8 source: its crates.io
# checksum covers build.rs and every DEPS file quoted below.
CRATE_VERSION=150.4.0
CRATE_SHA256=42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49
CRATE_URL=https://static.crates.io/crates/v8/v8-${CRATE_VERSION}.crate

# v8/DEPS `gn_version`: git_revision:3357c4f51b1a9e676378c695dd9c7e9911c35ee6
GN_URL=https://gn.googlesource.com/gn
GN_REV=3357c4f51b1a9e676378c695dd9c7e9911c35ee6

# v8/DEPS `third_party/rust`
RUST_URL=https://chromium.googlesource.com/chromium/src/third_party/rust
RUST_REV=26e8ff47f18a8d28d6187a04b6a16cb7332356f8

# v8/DEPS `third_party/icu`
ICU_URL=https://chromium.googlesource.com/chromium/deps/icu
ICU_REV=ee5f27adc28bd3f15b2c293f726d14d2e336cbd5
ICU_LITTLE_DAT_SHA256=1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612
ICU_BIG_DAT_SHA256=c58e1567b34e12a672b24ff4a5f21efa4c4481ab64a213dde059309b01aacde2

# v8/DEPS `third_party/rust-toolchain`, host_os == "linux"
TOOLCHAIN_SHA256=832de79f8d90940f4aaef023f83a00c1e7210c023f4d57f606b7bf9831c889aa
TOOLCHAIN_SIZE=274393124
TOOLCHAIN_NAME=rust-toolchain-4c4205163abcbd08948b3efab796c543ba1ea687-4-llvmorg-23-init-10931-g20b6ec66.tar.xz
TOOLCHAIN_URL=https://storage.googleapis.com/chromium-browser-clang/Linux_x64/${TOOLCHAIN_NAME}

# v8/DEPS `third_party/llvm-build/Release+Asserts`, host_os == "linux"
CLANG_SHA256=de584381536aa5ba2403033c4f8b70f3c39c2e5d7fa87c953b7fd8bfbba0ee2a
CLANG_SIZE=69635068
CLANG_NAME=clang-llvmorg-23-init-10931-g20b6ec66-11.tar.xz
CLANG_URL=https://storage.googleapis.com/chromium-browser-clang/Linux_x64/${CLANG_NAME}

# --- Layout ----------------------------------------------------------------
SRC=$CACHE/src
DOWNLOADS=$CACHE/downloads
WORK=$CACHE/work
V8_ROOT=$SRC/v8
V8_DIR=$V8_ROOT/v8-${CRATE_VERSION}
GN_DIR=$SRC/gn
RUST_DIR=$SRC/rust
ICU_DIR=$SRC/icu
GN_BIN=$GN_DIR/out/gn
CLANG_DIR=$WORK/clang
TOOLCHAIN_DIR=$V8_DIR/third_party/rust-toolchain

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "missing host tool: $1"
}

sha256_of() {
    sha256sum "$1" | cut -d' ' -f1
}

size_of() {
    stat -c %s "$1"
}

# Non-fatal predicate: does this file exist with exactly this hash/size?
matches_file() {
    local path=$1 want_sha=$2 want_size=${3:-}
    [ -f "$path" ] || return 1
    [ -z "$want_size" ] || [ "$(size_of "$path")" = "$want_size" ] || return 1
    [ "$(sha256_of "$path")" = "$want_sha" ]
}

# Fatal check: used after a download and for verified upstream sources.
check_file() {
    local path=$1 want_sha=$2 want_size=${3:-}
    [ -f "$path" ] || die "missing file: $path"
    if [ -n "$want_size" ]; then
        [ "$(size_of "$path")" = "$want_size" ] ||
            die "size mismatch for $path: got $(size_of "$path"), want $want_size"
    fi
    [ "$(sha256_of "$path")" = "$want_sha" ] ||
        die "sha256 mismatch for $path"
}

download() {
    local url=$1 out=$2 sha=$3 size=${4:-}
    if matches_file "$out" "$sha" "$size"; then
        return 0
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified: $out"
    mkdir -p "$(dirname "$out")"
    printf 'fetch %s\n' "$url"
    curl -fL --retry 3 --connect-timeout 30 -o "$out.part" "$url"
    check_file "$out.part" "$sha" "$size"
    mv "$out.part" "$out"
}

fetch_commit() {
    local name=$1 url=$2 rev=$3 dir=$CACHE/src/$1
    if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null || true)" = "$rev" ]; then
        return 0
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified git checkout: $dir @ $rev"
    printf 'fetch %s @ %s\n' "$url" "$rev"
    rm -rf "$dir"
    mkdir -p "$dir"
    git -C "$dir" init -q
    git -C "$dir" remote add origin "$url"
    GIT_TERMINAL_PROMPT=0 git -C "$dir" fetch -q --depth 1 origin "$rev"
    git -C "$dir" checkout -q FETCH_HEAD
    [ "$(git -C "$dir" rev-parse HEAD)" = "$rev" ] ||
        die "commit mismatch for $dir: got $(git -C "$dir" rev-parse HEAD), want $rev"
}

# --- Host tools ------------------------------------------------------------
need git
need curl
need sha256sum
need tar
need python3
need ninja
command -v clang++ >/dev/null 2>&1 || command -v g++ >/dev/null 2>&1 ||
    die "missing host tool: clang++ or g++"

# --- Downloads -------------------------------------------------------------
download "$TOOLCHAIN_URL" "$DOWNLOADS/$TOOLCHAIN_NAME" "$TOOLCHAIN_SHA256" "$TOOLCHAIN_SIZE"
download "$CLANG_URL" "$DOWNLOADS/$CLANG_NAME" "$CLANG_SHA256" "$CLANG_SIZE"

# The crate archive. Prefer the copy cargo already verified; otherwise fetch
# the same object from crates.io and check it against Cargo.lock's checksum.
CRATE_FILE=$DOWNLOADS/v8-${CRATE_VERSION}.crate
shopt -s nullglob
for candidate in "$HOME"/.cargo/registry/cache/*/"v8-$CRATE_VERSION.crate"; do
    if matches_file "$candidate" "$CRATE_SHA256"; then
        CRATE_FILE=$candidate
        break
    fi
done
shopt -u nullglob
download "$CRATE_URL" "$CRATE_FILE" "$CRATE_SHA256"

fetch_commit gn "$GN_URL" "$GN_REV"
fetch_commit rust "$RUST_URL" "$RUST_REV"
fetch_commit icu "$ICU_URL" "$ICU_REV"

# --- Unpack and overlay ----------------------------------------------------
if [ ! -d "$V8_DIR" ]; then
    [ "$VERIFY_ONLY" = 0 ] || die "missing crate tree: $V8_DIR"
    mkdir -p "$V8_ROOT"
    tar -xzf "$CRATE_FILE" -C "$V8_ROOT"
fi

mkdir -p "$V8_DIR/third_party/icu/common"
for pair in "icudtl.dat:$ICU_LITTLE_DAT_SHA256" "icudtb.dat:$ICU_BIG_DAT_SHA256"; do
    name=${pair%%:*}
    want=${pair#*:}
    dst=$V8_DIR/third_party/icu/common/$name
    src=$ICU_DIR/common/$name
    if matches_file "$dst" "$want"; then
        continue
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified: $dst"
    check_file "$src" "$want"
    cp -f "$src" "$dst"
done

# The crate ships only the per-crate BUILD.gn stubs; GN needs the vendor tree.
if [ ! -f "$V8_DIR/third_party/rust/chromium_crates_io/Cargo.lock" ]; then
    [ "$VERIFY_ONLY" = 0 ] || die "missing third_party/rust vendor tree"
    rm -rf "$V8_DIR/third_party/rust"
    cp -a "$RUST_DIR" "$V8_DIR/third_party/rust"
    rm -rf "$V8_DIR/third_party/rust/.git"
fi

if [ ! -x "$TOOLCHAIN_DIR/bin/rustc" ]; then
    [ "$VERIFY_ONLY" = 0 ] || die "missing Rust toolchain: $TOOLCHAIN_DIR"
    mkdir -p "$TOOLCHAIN_DIR"
    tar -xJf "$DOWNLOADS/$TOOLCHAIN_NAME" -C "$TOOLCHAIN_DIR"
fi
# build.rs skips its own download only when this sentinel matches the URL.
if [ "$VERIFY_ONLY" = 0 ]; then
    printf '%s' "$TOOLCHAIN_URL" > "$TOOLCHAIN_DIR/.rusty_v8_version"
elif ! grep -qxF "$TOOLCHAIN_URL" "$TOOLCHAIN_DIR/.rusty_v8_version" 2>/dev/null; then
    die "Rust toolchain sentinel is missing or stale: $TOOLCHAIN_DIR/.rusty_v8_version"
fi

if [ ! -x "$GN_BIN" ]; then
    [ "$VERIFY_ONLY" = 0 ] || die "missing GN binary: $GN_BIN"
    # `git describe --match initial-commit` needs the root commit, so the
    # shallow fetch above is not enough for GN's own version stamp.
    if [ "$(git -C "$GN_DIR" rev-parse --is-shallow-repository)" = true ]; then
        git -C "$GN_DIR" fetch -q --unshallow origin
    fi
    (
        cd "$GN_DIR"
        python3 build/gen.py --out-path out
        ninja -C out gn
    )
fi
case $("$GN_BIN" --version) in
    *"(${GN_REV:0:12})"*) ;;
    *) die "GN version does not report ${GN_REV:0:12}: $("$GN_BIN" --version)" ;;
esac

if [ ! -x "$CLANG_DIR/bin/clang" ]; then
    [ "$VERIFY_ONLY" = 0 ] || die "missing Chromium clang: $CLANG_DIR"
    mkdir -p "$CLANG_DIR"
    tar -xJf "$DOWNLOADS/$CLANG_NAME" -C "$CLANG_DIR"
fi

# --- Report ----------------------------------------------------------------
if [ "$VERIFY_ONLY" = 0 ]; then
    mkdir -p "$WORK"
    cat > "$WORK/build-env.sh" <<EOF
# Generated by eng/v8-source/prepare.sh. Source this before a from-source build.
export V8_FROM_SOURCE=1
export GN=$GN_BIN
export NINJA=$(command -v ninja)
export CLANG_BASE_PATH=$CLANG_DIR
export CARGO_TARGET_DIR=$WORK/target
EOF
    if [ -f /usr/lib/libclang.so ]; then
        printf 'export LIBCLANG_PATH=/usr/lib\n' >> "$WORK/build-env.sh"
    fi
fi

printf 'verified v8 %s inputs in %s\n' "$CRATE_VERSION" "$CACHE"
printf '  gn     %s (%s)\n' "$("$GN_BIN" --version)" "$(sha256_of "$GN_BIN")"
printf '  rustc  %s\n' "$("$TOOLCHAIN_DIR/bin/rustc" --version)"
printf '  clang  %s\n' "$("$CLANG_DIR/bin/clang" --version | head -1)"
[ "$VERIFY_ONLY" = 0 ] && printf '  env    %s\n' "$WORK/build-env.sh"
printf 'ok\n'
