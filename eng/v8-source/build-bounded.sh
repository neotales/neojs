#!/usr/bin/env bash
# Build a v8 150.4.0 / V8 15.0.245.2 archive with the narrow bounded own-key
# API used by neojs-node. Inputs come from an independently verified prepared
# cache; the V8 tree and archive are never committed.
#
# Required inputs:
#   NEOTALES_V8_CACHE  prepared cache produced by eng/v8-source/prepare.sh
#   NEOTALES_V8_PROCEDURE  read-only procedure worktree containing prepare.sh
#
# Optional:
#   NEOJS_V8_BUILD_DIR  dedicated build root (default ~/.cache/neotales/neojs-node-v8-build)
#   NUM_JOBS            ninja/cargo parallelism (default 12)
set -euo pipefail

PROCEDURE=${NEOTALES_V8_PROCEDURE:-/home/dev/.local/share/opencode/worktree/e4b071/neojs-v8-build}
CACHE=${NEOTALES_V8_CACHE:?set NEOTALES_V8_CACHE to the verified prepared cache}
BUILD=${NEOJS_V8_BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-node-v8-build}
JOBS=${NUM_JOBS:-12}
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
PATCH=$SCRIPT_DIR/patches/0001-bounded-own-enumerable-string-keys.patch

[ -x "$PROCEDURE/eng/v8-source/prepare.sh" ] ||
    { echo "missing verified prepare.sh in $PROCEDURE" >&2; exit 1; }
[ -f "$PATCH" ] || { echo "missing $PATCH" >&2; exit 1; }

# The verify-only path performs no writes, no network, and executes no
# downloaded toolchain. It detects drift in the prepared inputs.
"$PROCEDURE/eng/v8-source/prepare.sh" --verify-only

SRC=$BUILD/src/v8-150.4.0
mkdir -p "$BUILD/src"
if [ ! -d "$SRC" ]; then
    cp -a --reflink=auto "$CACHE/src/v8/v8-150.4.0" "$SRC"
fi

if grep -q "neojs_v8_get_own_enumerable_string_keys_bounded" "$SRC/v8/src/api/api.cc"; then
    echo "patch already present in $SRC"
else
    patch -p1 -d "$SRC" < "$PATCH"
fi

BUILDER=$BUILD/builder
mkdir -p "$BUILDER/src"
cat > "$BUILDER/Cargo.toml" <<EOF
[package]
name = "neojs-v8-bounded-build"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
rusty_v8 = { package = "v8", path = "$SRC", default-features = false, features = ["simdutf", "use_custom_libcxx"] }
EOF
: > "$BUILDER/src/lib.rs"

# shellcheck disable=SC1091
source "$CACHE/work/build-env.sh"
export CARGO_TARGET_DIR=$BUILD/target
export NUM_JOBS=$JOBS
export V8_FROM_SOURCE=1
export GN NINJA CLANG_BASE_PATH LIBCLANG_PATH

cargo +1.100.0 build --release --offline --manifest-path "$BUILDER/Cargo.toml"

ARCHIVE=$BUILD/target/release/build/gn_out/obj/librusty_v8.a
[ -f "$ARCHIVE" ] || { echo "missing $ARCHIVE" >&2; exit 1; }
nm -g --defined-only "$ARCHIVE" > "$BUILD/symbols.txt"
grep -q neojs_v8_get_own_enumerable_string_keys_bounded "$BUILD/symbols.txt" ||
    { echo "bounded symbol missing from $ARCHIVE" >&2; exit 1; }

printf 'archive %s\n' "$ARCHIVE"
printf 'sha256  %s\n' "$(sha256sum "$ARCHIVE" | cut -d' ' -f1)"
printf 'Use RUSTY_V8_ARCHIVE=%s for the Node build and tests.\n' "$ARCHIVE"
