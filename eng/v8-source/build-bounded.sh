#!/usr/bin/env bash
# Build a v8 150.4.0 / V8 15.0.245.2 archive with the narrow bounded own-key
# API used by neojs-node. The V8 tree and archive are never committed.
#
# Trust boundary: NEOTALES_V8_CACHE and NEOTALES_V8_PROCEDURE are caller-
# supplied roots. `prepare.sh --verify-only` is drift detection, not attacker
# authentication. This script verifies the patch and post-patch source hashes
# below, and uses the pinned builder.lock.
#
# Usage:
#   NEOTALES_V8_CACHE=/verified/cache \
#   NEOTALES_V8_PROCEDURE=/read-only/procedure \
#   eng/v8-source/build-bounded.sh [--prepare-only]
#
# The build root is recreated from the verified source on every run. It must
# not overlap the cache, procedure, or repository.
set -euo pipefail

PROCEDURE=${NEOTALES_V8_PROCEDURE:?set NEOTALES_V8_PROCEDURE to the read-only procedure worktree}
CACHE=${NEOTALES_V8_CACHE:?set NEOTALES_V8_CACHE to the verified prepared cache}
BUILD=${NEOJS_V8_BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-node-v8-build}
JOBS=${NUM_JOBS:-12}
PREPARE_ONLY=0
case ${1:-} in
    --prepare-only) PREPARE_ONLY=1 ;;
    "") ;;
    *) echo "usage: $0 [--prepare-only]" >&2; exit 2 ;;
esac

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../.." && pwd)
PATCH=$SCRIPT_DIR/patches/0001-bounded-own-enumerable-string-keys.patch
PATCH_SHA=f5933d7169a092eb03f909138a76bad492e65c46da90b70ea50621622c1646ca
KEYS_H_SHA=29e3de9dcb8e3c4424a3bc6b3520573cae6c1bdff29104bbbdb20e6a298e1890
KEYS_CC_SHA=ca0260e09da2e91cbf979bee393da555fb0d26056b6075d22f4d11a750873561
API_CC_SHA=be4d4ac17b905bc02cb6a0f9464b24a43df03649ff96c6356c5b9e26bcb344f4
BUILDER_LOCK=$SCRIPT_DIR/builder.lock

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
canon() { realpath -m "$1"; }
check_file() {
    local path=$1 want=$2
    [ -f "$path" ] || die "missing file: $path"
    [ "$(sha256sum "$path" | cut -d' ' -f1)" = "$want" ] ||
        die "sha256 mismatch for $path"
}
overlaps() {
    case "$1/" in "$2/"*) return 0 ;; esac
    case "$2/" in "$1/"*) return 0 ;; esac
    return 1
}

[ -x "$PROCEDURE/eng/v8-source/prepare.sh" ] ||
    die "missing verified prepare.sh in $PROCEDURE"
[ -f "$PATCH" ] || die "missing $PATCH"
[ -f "$BUILDER_LOCK" ] || die "missing $BUILDER_LOCK"

BUILD=$(canon "$BUILD")
CACHE=$(canon "$CACHE")
PROCEDURE=$(canon "$PROCEDURE")
REPO=$(canon "$REPO")
for other in "$CACHE" "$PROCEDURE" "$REPO"; do
    if overlaps "$BUILD" "$other"; then
        die "build root $BUILD overlaps $other"
    fi
done

[ "$BUILD" != "/" ] || die "build root must not be /"
mkdir -p "$BUILD"
command -v flock >/dev/null 2>&1 || die "flock is required to isolate concurrent builds"
exec 9>"$BUILD/.build.lock"
flock -n 9 || die "another build holds $BUILD/.build.lock"

# Read-only verification of the prepared inputs.
"$PROCEDURE/eng/v8-source/prepare.sh" --verify-only

# Fresh isolated source every run.
mkdir -p "$BUILD/src"
SRC=$BUILD/src/v8-150.4.0
rm -rf "$SRC"
cp -a --reflink=auto "$CACHE/src/v8/v8-150.4.0" "$SRC"

check_file "$PATCH" "$PATCH_SHA"
patch --fuzz=0 -p1 -d "$SRC" < "$PATCH"
check_file "$SRC/v8/src/objects/keys.h" "$KEYS_H_SHA"
check_file "$SRC/v8/src/objects/keys.cc" "$KEYS_CC_SHA"
check_file "$SRC/v8/src/api/api.cc" "$API_CC_SHA"

if [ "$PREPARE_ONLY" = 1 ]; then
    printf 'prepared %s with exact bounded-keys patch\n' "$SRC"
    exit 0
fi

RUN_ID=$(date -u +%Y%m%dT%H%M%SZ)-$$
TARGET=$BUILD/targets/$RUN_ID
BUILDER=$BUILD/builders/$RUN_ID
ARTIFACT=$BUILD/artifacts/$RUN_ID
mkdir -p "$TARGET" "$BUILDER/src" "$ARTIFACT"
python3 - "$SRC" > "$BUILDER/Cargo.toml" <<'PY'
import json, sys
src = sys.argv[1]
if any(ord(ch) < 0x20 for ch in src) or any(0xD800 <= ord(ch) <= 0xDFFF for ch in src):
    raise SystemExit("build path contains a control character or unpaired surrogate")
src.encode("utf-8")
print(f"""[package]
name = "neojs-v8-bounded-build"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
rusty_v8 = {{ package = "v8", path = {json.dumps(src, ensure_ascii=False)}, default-features = false, features = ["simdutf", "use_custom_libcxx"] }}
""")
PY
: > "$BUILDER/src/lib.rs"
cp "$BUILDER_LOCK" "$BUILDER/Cargo.lock"
grep -q 'name = "v8"' "$BUILDER/Cargo.lock" || die "builder.lock missing v8"
grep -A1 'name = "v8"' "$BUILDER/Cargo.lock" | grep -q 'version = "150.4.0"' ||
    die "builder.lock does not pin v8 150.4.0"
grep -A1 'name = "bindgen"' "$BUILDER/Cargo.lock" | grep -q 'version = "0.72.1"' ||
    die "builder.lock does not pin bindgen 0.72.1"

# shellcheck disable=SC1091
source "$CACHE/work/build-env.sh"
export CARGO_TARGET_DIR=$TARGET
export NUM_JOBS=$JOBS
export V8_FROM_SOURCE=1

cargo +1.100.0 build --release --locked --offline --manifest-path "$BUILDER/Cargo.toml"
cargo +1.100.0 tree --locked --offline --manifest-path "$BUILDER/Cargo.toml" -e normal,build > "$BUILD/tree.txt"
grep -q 'v8 v150.4.0' "$BUILD/tree.txt" || die "resolved v8 is not 150.4.0"
grep -q 'bindgen v0.72.1' "$BUILD/tree.txt" || die "resolved bindgen is not 0.72.1"

ARCHIVE=$TARGET/release/build/gn_out/obj/librusty_v8.a
[ -f "$ARCHIVE" ] || die "missing $ARCHIVE"
nm -g --defined-only "$ARCHIVE" > "$BUILD/symbols.txt"
grep -q neojs_v8_get_own_enumerable_string_keys_bounded "$BUILD/symbols.txt" ||
    die "bounded symbol missing from $ARCHIVE"

cp "$ARCHIVE" "$ARTIFACT/librusty_v8.a"
ARCHIVE_SHA=$(sha256sum "$ARTIFACT/librusty_v8.a" | cut -d' ' -f1)
ARCHIVE_SIZE=$(stat -c %s "$ARTIFACT/librusty_v8.a")
PROCEDURE_REV=$(git -C "$PROCEDURE" rev-parse HEAD)
LOCK_SHA=$(sha256sum "$BUILDER_LOCK" | cut -d' ' -f1)
cat > "$ARTIFACT/provenance.txt" <<EOF
built_utc=$RUN_ID
v8_crate=150.4.0
v8_version=15.0.245.2
patch_sha256=$PATCH_SHA
patched_keys_h=$KEYS_H_SHA
patched_keys_cc=$KEYS_CC_SHA
patched_api_cc=$API_CC_SHA
builder_lock_sha256=$LOCK_SHA
procedure_commit=$PROCEDURE_REV
cache=$CACHE
archive_sha256=$ARCHIVE_SHA
archive_size=$ARCHIVE_SIZE
EOF
printf 'archive %s\n' "$ARTIFACT/librusty_v8.a"
printf 'sha256  %s\n' "$ARCHIVE_SHA"
printf 'provenance %s\n' "$ARTIFACT/provenance.txt"
printf 'Use RUSTY_V8_ARCHIVE=%s for the Node build and tests.\n' "$ARTIFACT/librusty_v8.a"
printf 'After rebuilding, run `cargo clean -p v8` in the consumer target so the\n'
printf 'new archive is relinked; Cargo does not track the archive file content.\n'
