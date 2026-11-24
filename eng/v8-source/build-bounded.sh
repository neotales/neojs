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
#   eng/v8-source/build-bounded.sh [--prepare-only | --run-root-only | --check-procedure-only]
#
# The build root is recreated from the verified source on every run. It must
# not overlap the cache, procedure, or repository. Every run gets a unique
# `mktemp -d` run root; target and artifact directories are never reused.
set -euo pipefail

PROCEDURE=${NEOTALES_V8_PROCEDURE:?set NEOTALES_V8_PROCEDURE to the read-only procedure worktree}
CACHE=${NEOTALES_V8_CACHE:?set NEOTALES_V8_CACHE to the verified prepared cache}
BUILD=${NEOJS_V8_BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-node-v8-build}
JOBS=${NUM_JOBS:-12}
MODE=build
EXPECT_REV=
EXPECT_HELPER_TREE=
EXPECT_HELPER_FILES=
while [ $# -gt 0 ]; do
    case $1 in
        --prepare-only) MODE=prepare ;;
        --run-root-only) MODE=run-root ;;
        --check-procedure-only) MODE=check-procedure ;;
        --expect-rev)
            [ $# -ge 2 ] || { echo "usage: $0 --expect-rev needs a value" >&2; exit 2; }
            EXPECT_REV=$2
            shift
            ;;
        --expect-helper-tree)
            [ $# -ge 2 ] || { echo "usage: $0 --expect-helper-tree needs a value" >&2; exit 2; }
            EXPECT_HELPER_TREE=$2
            shift
            ;;
        --expect-helper-files)
            [ $# -ge 2 ] || { echo "usage: $0 --expect-helper-files needs a value" >&2; exit 2; }
            EXPECT_HELPER_FILES=$2
            shift
            ;;
        *) echo "usage: $0 [--prepare-only | --run-root-only | --check-procedure-only] [--expect-rev REV] [--expect-helper-tree TREE] [--expect-helper-files N]" >&2; exit 2 ;;
    esac
    shift
done

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SCRIPT_DIR/../.." && pwd)
PATCH=$SCRIPT_DIR/patches/0001-bounded-own-enumerable-string-keys.patch
PATCH_SHA=addc1c8b41a614911a4f1c8905780c3dffbaa3888ecdb159d8a444bc71ae044c
KEYS_H_SHA=5fbf2e84da60df7f0b7b8e35d8dc73d261ab44043e6fe9e3cef6835cfd0d4179
KEYS_CC_SHA=262f305d6b7918a594362ea829ce074f51a862272cc1c631da102e5bf765e56a
API_CC_SHA=a96c9c0c8b575496da039d6abc98f4537a274f33d994946a851744faed5159ce
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

new_run_root() {
    local runs=$BUILD/runs
    mkdir -p "$runs" || die "cannot create $runs"
    mktemp -d "$runs/$(date -u +%Y%m%dT%H%M%SZ).XXXXXX" ||
        die "cannot create a unique run root under $runs"
}

# Verify the exact procedure code that this script is about to execute.
# `git status` without --ignored does not see ignored helpers, so check the
# helper directory with --ignored and compare every HEAD helper's bytes and
# executable bit. The recorded commit is the identity, not a signature.
verify_procedure() {
    local expected_rev=${1:-}
    local expected_tree=${2:-}
    local expected_files=${3:-}
    [ -x "$PROCEDURE/eng/v8-source/prepare.sh" ] ||
        die "missing verified prepare.sh in $PROCEDURE"
    local rev
    rev=$(git -C "$PROCEDURE" rev-parse HEAD) ||
        die "cannot read procedure HEAD in $PROCEDURE"
    local status
    status=$(git -C "$PROCEDURE" status --porcelain --untracked-files=normal) ||
        die "cannot inspect procedure worktree $PROCEDURE"
    if [ -n "$status" ]; then
        printf 'dirty procedure entries:\n%s\n' "$status" >&2
        die "procedure worktree $PROCEDURE must be clean for provenance"
    fi
    status=$(git -C "$PROCEDURE" status --porcelain --untracked-files=all --ignored -- eng/v8-source) ||
        die "cannot inspect procedure helpers in $PROCEDURE"
    if [ -n "$status" ]; then
        printf 'procedure helper entries:\n%s\n' "$status" >&2
        die "procedure helper directory has untracked, ignored, staged, or modified entries"
    fi

    local tree
    tree=$(git -C "$PROCEDURE" rev-parse "$rev:eng/v8-source") ||
        die "procedure has no eng/v8-source tree at $rev"
    for helper in prepare.sh validate_tar.py verify_git_tree.py materialize_git_tree.py; do
        git -C "$PROCEDURE" cat-file -e "$rev:eng/v8-source/$helper" 2>/dev/null ||
            die "procedure helper is not tracked at $rev: $helper"
    done

    local list=$BUILD/.procedure-helpers.$$
    git -C "$PROCEDURE" ls-tree -r -z "$rev" -- eng/v8-source > "$list" ||
        die "cannot list procedure helpers in $PROCEDURE"
    local files=0
    local record meta path mode rest type blob actual
    while IFS= read -r -d '' record; do
        meta=${record%%$'\t'*}
        path=${record#*$'\t'}
        mode=${meta%% *}
        rest=${meta#* }
        type=${rest%% *}
        blob=${rest#* }
        [ "$type" = blob ] || die "procedure helper is not a blob: $path"
        actual=$PROCEDURE/$path
        [ -f "$actual" ] || die "procedure helper missing or not a regular file: $path"
        [ ! -L "$actual" ] || die "procedure helper is a symlink: $path"
        case $mode in
            100755) [ -x "$actual" ] || die "procedure helper is not executable: $path" ;;
            100644) [ ! -x "$actual" ] || die "procedure helper is unexpectedly executable: $path" ;;
            *) die "procedure helper has unsupported git mode $mode: $path" ;;
        esac
        git -C "$PROCEDURE" cat-file blob "$blob" | cmp -s - "$actual" ||
            die "procedure helper does not match $rev: $path"
        files=$((files + 1))
    done < "$list"
    rm -f "$list"
    [ "$files" -gt 0 ] || die "procedure helper directory is empty"

    if [ -n "$expected_rev" ] && [ "$rev" != "$expected_rev" ]; then
        die "procedure revision changed during the build: got $rev, expected $expected_rev"
    fi
    if [ -n "$expected_tree" ] && [ "$tree" != "$expected_tree" ]; then
        die "procedure helper tree changed during the build: got $tree, expected $expected_tree"
    fi
    if [ -n "$expected_files" ] && [ "$files" != "$expected_files" ]; then
        die "procedure helper file count changed during the build: got $files, expected $expected_files"
    fi

    PROCEDURE_REV=$rev
    PROCEDURE_HELPER_TREE=$tree
    PROCEDURE_HELPER_FILES=$files
}

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

if [ "$MODE" = run-root ]; then
    RUN_ROOT=$(new_run_root)
    printf 'run_root %s\n' "$RUN_ROOT"
    exit 0
fi

if [ "$MODE" = check-procedure ]; then
    verify_procedure "$EXPECT_REV" "$EXPECT_HELPER_TREE" "$EXPECT_HELPER_FILES"
    printf 'procedure %s helper-tree %s files %s\n' \
        "$PROCEDURE_REV" "$PROCEDURE_HELPER_TREE" "$PROCEDURE_HELPER_FILES"
    exit 0
fi

verify_procedure
PROCEDURE_REV_BEFORE=$PROCEDURE_REV
PROCEDURE_HELPER_TREE_BEFORE=$PROCEDURE_HELPER_TREE
PROCEDURE_HELPER_FILES_BEFORE=$PROCEDURE_HELPER_FILES

[ -f "$PATCH" ] || die "missing $PATCH"
[ -f "$BUILDER_LOCK" ] || die "missing $BUILDER_LOCK"

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

if [ "$MODE" = prepare ]; then
    printf 'prepared %s with exact bounded-keys patch\n' "$SRC"
    exit 0
fi

RUN_ROOT=$(new_run_root)
RUN_ID=$(basename "$RUN_ROOT")
BUILT_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
TARGET=$RUN_ROOT/target
BUILDER=$RUN_ROOT/builder
ARTIFACT=$RUN_ROOT/artifact
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

cargo +1.100.0 build -j "$JOBS" --release --locked --offline --manifest-path "$BUILDER/Cargo.toml"
cargo +1.100.0 tree --locked --offline --manifest-path "$BUILDER/Cargo.toml" -e normal,build > "$RUN_ROOT/tree.txt"
grep -q 'v8 v150.4.0' "$RUN_ROOT/tree.txt" || die "resolved v8 is not 150.4.0"
grep -q 'bindgen v0.72.1' "$RUN_ROOT/tree.txt" || die "resolved bindgen is not 0.72.1"

ARCHIVE=$TARGET/release/build/gn_out/obj/librusty_v8.a
[ -f "$ARCHIVE" ] || die "missing $ARCHIVE"
nm -g --defined-only "$ARCHIVE" > "$RUN_ROOT/symbols.txt"
grep -q neojs_v8_get_own_enumerable_string_keys_bounded "$RUN_ROOT/symbols.txt" ||
    die "bounded symbol missing from $ARCHIVE"

cp "$ARCHIVE" "$ARTIFACT/librusty_v8.a"
ARCHIVE_SHA=$(sha256sum "$ARTIFACT/librusty_v8.a" | cut -d' ' -f1)
ARCHIVE_SIZE=$(stat -c %s "$ARTIFACT/librusty_v8.a")
# Re-check after the long build and fail if the procedure switched to another
# clean commit. Do not let the post-check overwrite the pre-build identity
# recorded in provenance.
verify_procedure "$PROCEDURE_REV_BEFORE" "$PROCEDURE_HELPER_TREE_BEFORE" "$PROCEDURE_HELPER_FILES_BEFORE"
LOCK_SHA=$(sha256sum "$BUILDER_LOCK" | cut -d' ' -f1)
cat > "$ARTIFACT/provenance.txt" <<EOF
built_utc=$BUILT_UTC
run_id=$RUN_ID
run_root=$RUN_ROOT
v8_crate=150.4.0
v8_version=15.0.245.2
patch_sha256=$PATCH_SHA
patched_keys_h=$KEYS_H_SHA
patched_keys_cc=$KEYS_CC_SHA
patched_api_cc=$API_CC_SHA
builder_lock_sha256=$LOCK_SHA
procedure_commit=$PROCEDURE_REV_BEFORE
procedure_status=clean
procedure_helpers=verified
procedure_helper_tree=$PROCEDURE_HELPER_TREE_BEFORE
procedure_helper_files=$PROCEDURE_HELPER_FILES_BEFORE
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
