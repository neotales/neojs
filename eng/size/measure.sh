#!/usr/bin/env bash
# Measure release-profile vaeljs and vaeljs-lite binaries and append the result
# to the size ledger.
#
# The script does not copy, strip, compress or install the binary. It reports
# the on-disk length and SHA-256 of the executable Cargo produced. Re-running
# the same measurement is a no-op; a missing package is recorded explicitly
# and makes the run fail unless --allow-missing is passed.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

ROOT=$(size_repo_root)
DEFAULT_LEDGER="$ROOT/eng/size/measurements.jsonl"
DEFAULT_TARGET_DIR="$ROOT/target/size"
TOOLCHAIN_FILE="$SCRIPT_DIR/toolchain"

size_require jq sha256sum flock git cargo rustc cc c++ realpath mktemp

usage() {
    cat <<'USAGE'
Usage: eng/size/measure.sh [options]

Measures release-profile workspace binaries and records one JSON object per
artifact in eng/size/measurements.jsonl (or --ledger). The default packages
are vaeljs and vaeljs-lite.

Options:
  --package NAME          Measure NAME; repeat for more than one package.
                          Default: vaeljs and vaeljs-lite.
  --target TRIPLE         Cargo target triple. Default: the rustc host.
  --toolchain NAME        rustup toolchain. Default: eng/size/toolchain.
  --features LIST         Comma-separated Cargo features to enable.
  --no-default-features   Build without the package's default features.
  --ledger PATH           Append to PATH instead of eng/size/measurements.jsonl.
  --target-dir PATH       Cargo target directory. Default: target/size.
  --note TEXT             Note to attach to each record.
  --allow-missing         Record missing packages and exit 0 instead of 3.
  --allow-dirty           Record a non-canonical measurement from a dirty tree.
                          The default refuses to record a dirty tree.
  --allow-env-overrides   Allow artifact-affecting environment variables and
                          record them. Without this, such variables are refused.
  --online                Let Cargo use the network. Default is --offline.
  --dry-run               Print commands and missing packages; build nothing.
  --no-record             Build and print the record, but do not append it.
  -h, --help              Show this help.

Exit status: 0 success, 1 configuration/build/validation error, 2 usage error,
3 one or more requested artifacts are missing (unless --allow-missing).
USAGE
}

PACKAGES=(vaeljs vaeljs-lite)
LEDGER=$DEFAULT_LEDGER
TARGET_DIR=$DEFAULT_TARGET_DIR
TOOLCHAIN=""
TARGET=""
FEATURES=()
NO_DEFAULT_FEATURES=0
ALLOW_MISSING=0
ALLOW_DIRTY=0
ALLOW_ENV_OVERRIDES=0
ONLINE=0
DRY_RUN=0
NO_RECORD=0
NOTE=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --package)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            if [ "${PACKAGES[*]:-}" = "vaeljs vaeljs-lite" ]; then
                PACKAGES=()
            fi
            PACKAGES+=("$2")
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            TARGET=$2
            shift 2
            ;;
        --toolchain)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            TOOLCHAIN=$2
            shift 2
            ;;
        --features)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            FEATURES+=("$2")
            shift 2
            ;;
        --no-default-features)
            NO_DEFAULT_FEATURES=1
            shift
            ;;
        --ledger)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            LEDGER=$2
            shift 2
            ;;
        --target-dir)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            TARGET_DIR=$2
            shift 2
            ;;
        --note)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            NOTE=$2
            shift 2
            ;;
        --allow-missing)
            ALLOW_MISSING=1
            shift
            ;;
        --allow-dirty)
            ALLOW_DIRTY=1
            shift
            ;;
        --allow-env-overrides)
            ALLOW_ENV_OVERRIDES=1
            shift
            ;;
        --online)
            ONLINE=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --no-record)
            NO_RECORD=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'size: error: unknown argument: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[ "${#PACKAGES[@]}" -gt 0 ] || size_die "no packages requested"

# A relative --target-dir and --ledger are relative to the caller's working
# directory. Normalize them once; every later path and lock uses the absolute
# form, independent of the directory Cargo is run from.
TARGET_DIR=$(realpath -m -- "$TARGET_DIR")
LEDGER=$(realpath -m -- "$LEDGER")

if [ -z "$TOOLCHAIN" ]; then
    [ -f "$TOOLCHAIN_FILE" ] || size_die "toolchain file not found: $TOOLCHAIN_FILE"
    TOOLCHAIN=$(sed -e 's/#.*//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
        "$TOOLCHAIN_FILE" | sed '/^$/d' | head -n 1)
    [ -n "$TOOLCHAIN" ] || size_die "no toolchain pinned in $TOOLCHAIN_FILE"
fi

mkdir -p -- "$TARGET_DIR" "$TARGET_DIR/tmp"
# Cargo must use the dedicated directory, or the path the script measures and
# the path the build writes to diverge. TMPDIR keeps build scratch out of the
# shared temporary directory.
export CARGO_TARGET_DIR="$TARGET_DIR"
export TMPDIR="$TARGET_DIR/tmp"

# Serialize build+measure for one target directory. Two runs with different
# ledgers or packages can still share a target dir; without this lock they race
# on Cargo's artifact files and can measure a half-written binary. The lock is
# in the dedicated target directory, not in git.
BUILD_LOCK="$TARGET_DIR/.size-build.lock"
size_lock_target_dir "$BUILD_LOCK"

# Builds are compared with each other only when the inputs that can move the
# byte count are the same. Refuse the ambient ones instead of quietly
# attributing their effect to the commit.
ENV_OVERRIDES='{}'
check_ambient_var() {
    local name=$1 value
    value=${!name:-}
    [ -z "$value" ] && return 0
    if [ "$ALLOW_ENV_OVERRIDES" -ne 1 ]; then
        size_die "$name is set and can change the artifact; unset it or pass --allow-env-overrides (the value will be recorded)"
    fi
    ENV_OVERRIDES=$(jq -c --arg name "$name" --arg value "$value" \
        '. + {($name): $value}' <<<"$ENV_OVERRIDES")
}

for ambient in \
    CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS CARGO_BUILD_TARGET \
    RUSTFLAGS RUSTDOCFLAGS RUSTC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER \
    CC CXX CFLAGS CXXFLAGS AR RANLIB LDFLAGS LD \
    MACOSX_DEPLOYMENT_TARGET SOURCE_DATE_EPOCH \
    V8_FROM_SOURCE GN_ARGS EXTRA_GN_ARGS CLANG_BASE_PATH; do
    check_ambient_var "$ambient"
done
TARGET_SPECIFIC_ENV_NAMES=()
while IFS= read -r name; do
    [ -n "$name" ] || continue
    case "$name" in
        CARGO_TARGET_DIR) ;;
        CARGO_TARGET_*)
            TARGET_SPECIFIC_ENV_NAMES+=("$name")
            check_ambient_var "$name"
            ;;
        CARGO_PROFILE_RELEASE_* | CARGO_BUILD_*)
            check_ambient_var "$name"
            ;;
    esac
done < <(compgen -e | grep -E '^(CARGO_PROFILE_RELEASE_|CARGO_TARGET_|CARGO_BUILD_)' | sort || true)

if [ "${#TARGET_SPECIFIC_ENV_NAMES[@]}" -eq 0 ]; then
    TARGET_SPECIFIC_ENV_JSON='{"status":"absent","names":[]}'
else
    TARGET_SPECIFIC_ENV_JSON=$(printf '%s\n' "${TARGET_SPECIFIC_ENV_NAMES[@]}" |
        jq -R -s '{status: "captured", names: (split("\n") | map(select(length > 0)))}')
fi

CARGO_CONFIG_JSON=$(size_cargo_config_json "$ROOT")

V8_ARCHIVE_JSON='null'
V8_ARCHIVE_SUPPLIED=false
if [ -n "${RUSTY_V8_ARCHIVE:-}" ]; then
    V8_ARCHIVE_SUPPLIED=true
    size_require gzip
    [ -r "$RUSTY_V8_ARCHIVE" ] ||
        size_die "RUSTY_V8_ARCHIVE is set but not readable: $RUSTY_V8_ARCHIVE"
    # Compare the decompressed static archive, not the gzip wrapper: two
    # machines can gzip the same .a differently and still build the same
    # binary. The wrapper hash is kept for provenance but is not part of the
    # record key.
    #
    # A content hash proves which bytes were supplied. It does not prove those
    # bytes are the official upstream archive, that they were authenticated, or
    # that the build actually consumed them. The record says so: provenance is
    # an operator-supplied local file and authenticated is false.
    V8_ARCHIVE_JSON=$(jq -n \
        --arg name "$(basename -- "$RUSTY_V8_ARCHIVE")" \
        --arg sha256 "$(gzip -dc "$RUSTY_V8_ARCHIVE" | sha256sum | awk '{print $1}')" \
        --arg archive_sha256 "$(sha256sum "$RUSTY_V8_ARCHIVE" | awk '{print $1}')" \
        '{
          name: $name,
          sha256: $sha256,
          archive_sha256: $archive_sha256,
          provenance: "operator-supplied-local-file",
          authenticated: false
        }')
fi

RUSTC_VERSION=$(rustc "+$TOOLCHAIN" -V)
CARGO_VERSION=$(cargo "+$TOOLCHAIN" -V)
RUSTC_VV=$(rustc "+$TOOLCHAIN" -vV)
HOST=$(sed -n 's/^host: //p' <<<"$RUSTC_VV")
LLVM_VERSION=$(sed -n 's/^LLVM version: //p' <<<"$RUSTC_VV")
[ -n "$HOST" ] || size_die "could not determine the rustc host triple"
[ -n "$TARGET" ] || TARGET=$HOST

CC_VERSION=$(cc --version 2>/dev/null | head -n 1 || true)
CXX_VERSION=$(c++ --version 2>/dev/null | head -n 1 || true)
[ -n "$CC_VERSION" ] || CC_VERSION="unavailable"
[ -n "$CXX_VERSION" ] || CXX_VERSION="unavailable"

OS_NAME=$(uname -s)
OS_ARCH=$(uname -m)
OS_KERNEL=$(uname -r)

GIT_COMMIT=$(git -C "$ROOT" rev-parse HEAD)
GIT_BRANCH=$(git -C "$ROOT" symbolic-ref -q --short HEAD 2>/dev/null || echo detached)
GIT_DESCRIBE=$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)
GIT_TAGS_JSON=$(git -C "$ROOT" tag --points-at HEAD | jq -R -s 'split("\n") | map(select(length > 0))')

GIT_STATE_SHA256=$(size_git_state_sha256 "$ROOT" "$LEDGER")
if size_git_dirty "$ROOT" "$LEDGER"; then
    GIT_DIRTY=true
else
    GIT_DIRTY=false
fi
if [ "$GIT_DIRTY" = true ] && [ "$ALLOW_DIRTY" -ne 1 ] && [ "$DRY_RUN" -ne 1 ]; then
    size_die "source tree is dirty; commit or stash the change, or pass --allow-dirty to record a non-canonical measurement"
fi
if [ "$GIT_DIRTY" = true ] && [ "$DRY_RUN" -eq 1 ]; then
    size_warn "source tree is dirty; a real measurement would refuse to record it without --allow-dirty"
fi
if [ "$GIT_DIRTY" = false ]; then
    GIT_CANONICAL=true
else
    GIT_CANONICAL=false
fi
STATE_STATUS=captured
STATE_SHA256=$GIT_STATE_SHA256
if [ "$(jq 'length' <<<"$GIT_TAGS_JSON")" -gt 0 ]; then
    SOURCE_KIND=tag
else
    SOURCE_KIND=commit
fi

[ -f "$ROOT/Cargo.lock" ] || size_die "Cargo.lock not found in $ROOT"
LOCK_SHA256=$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')
WORKSPACE_PATH_SHA256=$(printf '%s' "$ROOT" | sha256sum | awk '{print $1}')

FEATURES_JSON='[]'
if [ "${#FEATURES[@]}" -gt 0 ]; then
    FEATURES_JSON=$(printf '%s\n' "${FEATURES[@]}" | tr ',' '\n' |
        sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | sed '/^$/d' |
        sort -u | jq -R -s 'split("\n") | map(select(length > 0))')
fi
FEATURES_CSV=$(jq -r 'join(",")' <<<"$FEATURES_JSON")

NO_DEFAULT_JSON=false
if [ "$NO_DEFAULT_FEATURES" -eq 1 ]; then
    NO_DEFAULT_JSON=true
fi

OFFLINE_JSON=true
METADATA_NETWORK=()
BUILD_NETWORK=()
if [ "$ONLINE" -eq 1 ]; then
    OFFLINE_JSON=false
else
    METADATA_NETWORK+=(--offline)
    BUILD_NETWORK+=(--offline)
fi

METADATA=""
load_metadata() {
    [ -n "$METADATA" ] && return 0
    local error_file="$TARGET_DIR/metadata.err"
    if ! METADATA=$(cargo "+$TOOLCHAIN" metadata --manifest-path "$ROOT/Cargo.toml" \
        --no-deps --format-version 1 --locked "${METADATA_NETWORK[@]}" 2>"$error_file"); then
        cat "$error_file" >&2
        size_die "cargo metadata failed; fix Cargo.toml/Cargo.lock before measuring"
    fi
}

MISSING_COUNT=0
MEASURED_COUNT=0

make_record() {
    local status=$1
    local package=$2
    local bin=$3
    local artifact_path=$4
    local bytes_json=$5
    local sha256=$6
    local file_type=$7
    local command=$8
    local missing_reason=$9
    local measured_at=${10}
    local notes_json=${11}
    local v8_archive_status=${12-not-applicable}
    local v8_archive_json=${13-'null'}
    local record

    record=$(jq -n \
        --arg status "$status" \
        --arg package "$package" \
        --arg bin "$bin" \
        --arg artifact_path "$artifact_path" \
        --argjson bytes "$bytes_json" \
        --arg sha256 "$sha256" \
        --arg file_type "$file_type" \
        --arg command "$command" \
        --arg missing_reason "$missing_reason" \
        --arg measured_at "$measured_at" \
        --argjson notes "$notes_json" \
        --arg git_commit "$GIT_COMMIT" \
        --arg git_branch "$GIT_BRANCH" \
        --argjson git_dirty "$GIT_DIRTY" \
        --argjson git_canonical "$GIT_CANONICAL" \
        --arg git_describe "$GIT_DESCRIBE" \
        --argjson git_tags "$GIT_TAGS_JSON" \
        --arg source_kind "$SOURCE_KIND" \
        --arg state_status "$STATE_STATUS" \
        --argjson state_sha256 "$(jq -Rn --arg v "$STATE_SHA256" 'if $v == "" then null else $v end')" \
        --arg target "$TARGET" \
        --arg host "$HOST" \
        --arg os "$OS_NAME" \
        --arg arch "$OS_ARCH" \
        --arg kernel "$OS_KERNEL" \
        --arg toolchain "$TOOLCHAIN" \
        --arg rustc "$RUSTC_VERSION" \
        --arg cargo "$CARGO_VERSION" \
        --arg llvm "$LLVM_VERSION" \
        --arg cc "$CC_VERSION" \
        --arg cxx "$CXX_VERSION" \
        --argjson requested_features "$FEATURES_JSON" \
        --argjson requested_no_default_features "$NO_DEFAULT_JSON" \
        --argjson offline "$OFFLINE_JSON" \
        --arg lock_sha256 "$LOCK_SHA256" \
        --arg workspace_path_sha256 "$WORKSPACE_PATH_SHA256" \
        --argjson cargo_config "$CARGO_CONFIG_JSON" \
        --argjson target_specific_env "$TARGET_SPECIFIC_ENV_JSON" \
        --arg v8_archive_status "$v8_archive_status" \
        --argjson v8_archive "$v8_archive_json" \
        --argjson env_overrides "$ENV_OVERRIDES" \
        '{
          schema: 3,
          status: $status,
          artifact: {
            package: $package,
            bin: $bin,
            kind: "release-binary",
            path: $artifact_path
          },
          source: {
            git_commit: $git_commit,
            git_branch: $git_branch,
            git_dirty: $git_dirty,
            git_canonical: $git_canonical,
            git_describe: $git_describe,
            git_tags: $git_tags,
            kind: $source_kind,
            measured_from: "worktree",
            state_status: $state_status,
            state_sha256: $state_sha256
          },
          platform: {
            target: $target,
            host: $host,
            os: $os,
            arch: $arch,
            kernel: $kernel
          },
          toolchain: {
            requested: $toolchain,
            rustc: $rustc,
            cargo: $cargo,
            llvm: $llvm,
            cc: $cc,
            cxx: $cxx
          },
          build: {
            profile: "release",
            requested_features: $requested_features,
            requested_no_default_features: $requested_no_default_features,
            effective_features: {status: "not-captured"},
            effective_linker: {status: "not-captured"},
            offline: $offline,
            locked: true,
            command: (if $status == "ok" then $command else "" end),
            cargo_lock_sha256: $lock_sha256,
            workspace_path_sha256: $workspace_path_sha256,
            cargo_config: $cargo_config,
            env_overrides: $env_overrides,
            target_specific_env: $target_specific_env,
            target_dir: {
              lock: "cooperating-measure-script-only",
              ownership: "exclusive-required",
              external_writer_safety: "not-claimed"
            },
            v8_archive_status: $v8_archive_status,
            v8_archive: $v8_archive
          },
          measurement: {
            method: (if $status == "ok" then "file-length-bytes" else null end),
            bytes: (if $status == "ok" then $bytes else null end),
            sha256: (if $status == "ok" then $sha256 else null end),
            file: (if $status == "ok" then $file_type else null end)
          },
          missing_reason: (if $missing_reason == "" then null else $missing_reason end),
          measured_at: $measured_at,
          notes: $notes
        }')

    size_finalize_record "$record"
}

# RUSTY_V8_ARCHIVE is an environment input only for a package whose dependency
# graph actually contains V8. Recording it for vaeljs-lite would claim an input
# the artifact never used and would split an otherwise identical series.
package_uses_v8() {
    local package=$1 tree
    if ! tree=$(cargo "+$TOOLCHAIN" tree --manifest-path "$ROOT/Cargo.toml" \
        -p "$package" --prefix none --locked "${METADATA_NETWORK[@]}" 2>/dev/null); then
        size_die "cargo tree failed for $package; cannot attribute the V8 archive"
    fi
    grep -Eq '^(v8|deno_v8) v' <<<"$tree"
}

record_missing() {
    local package=$1
    local reason=$2
    local bin=$3
    local measured_at notes_json record
    measured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    notes_json=$(jq -Rn --arg note "$NOTE" 'if $note == "" then null else $note end')
    record=$(make_record "missing" "$package" "$bin" "$bin" "null" "" "" "" \
        "$reason" "$measured_at" "$notes_json" "not-applicable" "null")
    if [ "$NO_RECORD" -eq 1 ] || [ "$DRY_RUN" -eq 1 ]; then
        jq -c . <<<"$record"
    else
        size_append_record "$LEDGER" "$record"
    fi
}

measure_package() {
    local package=$1
    local bin=$2
    local binary
    local build_cmd
    local command
    local log
    local bytes sha256 file_type measured_at notes_json record
    local v8_json="null" v8_status="not-applicable"
    local before_commit before_state after_commit after_state

    build_cmd=(cargo "+$TOOLCHAIN" build \
        --release --locked --target "$TARGET" -p "$package")
    if [ "${#BUILD_NETWORK[@]}" -gt 0 ]; then
        build_cmd+=("${BUILD_NETWORK[@]}")
    fi
    if [ "$NO_DEFAULT_FEATURES" -eq 1 ]; then
        build_cmd+=(--no-default-features)
    fi
    if [ -n "$FEATURES_CSV" ]; then
        build_cmd+=(--features "$FEATURES_CSV")
    fi

    binary="$TARGET_DIR/$TARGET/release/$bin"
    if package_uses_v8 "$package"; then
        if [ "$V8_ARCHIVE_SUPPLIED" = true ]; then
            v8_status="supplied-provenance-unknown"
            v8_json=$V8_ARCHIVE_JSON
        else
            v8_status="not-supplied-not-captured"
        fi
    fi
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'size: dry run: %s %s\n' "$package" "$(printf '%q ' "${build_cmd[@]}")"
        printf 'size: dry run: artifact would be %s\n' "$binary"
        printf 'size: dry run: v8 archive status: %s\n' "$v8_status"
        printf 'size: dry run: v8 archive input: %s\n' "$v8_json"
        return 0
    fi

    command=$(printf '%q ' "${build_cmd[@]}")
    command=${command% }
    log="$TARGET_DIR/build-$package.log"
    before_commit=$(git -C "$ROOT" rev-parse HEAD)
    before_state=$(size_git_state_sha256 "$ROOT" "$LEDGER")
    printf 'size: building %s (%s)\n' "$package" "$TARGET" >&2
    if ! (cd -- "$ROOT" && "${build_cmd[@]}") >"$log" 2>&1; then
        tail -n 50 "$log" >&2
        size_die "build failed for $package; full log: $log"
    fi
    after_commit=$(git -C "$ROOT" rev-parse HEAD)
    after_state=$(size_git_state_sha256 "$ROOT" "$LEDGER")
    if [ "$before_commit" != "$GIT_COMMIT" ]; then
        size_die "commit changed after the initial source-state capture and before building $package; refusing to record the binary"
    fi
    if [ "$before_commit" != "$after_commit" ] || [ "$before_state" != "$after_state" ]; then
        size_die "source commit or dirty state changed while building $package; refusing to record a binary that cannot be attributed to the captured source state"
    fi
    if [ "$before_state" != "$STATE_SHA256" ]; then
        size_die "source state changed before building $package; refusing to record it"
    fi

    [ -f "$binary" ] || size_die "build succeeded but binary is missing: $binary"
    bytes=$(wc -c <"$binary" | tr -d '[:space:]')
    sha256=$(sha256sum "$binary" | awk '{print $1}')
    if command -v file >/dev/null 2>&1; then
        file_type=$(file -b "$binary")
    else
        file_type=""
    fi
    measured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    notes_json=$(jq -Rn --arg note "$NOTE" 'if $note == "" then null else $note end')
    record=$(make_record "ok" "$package" "$bin" "$bin" "$bytes" "$sha256" \
        "$file_type" "$command" "" "$measured_at" "$notes_json" "$v8_status" "$v8_json")

    MEASURED_COUNT=$((MEASURED_COUNT + 1))
    if [ "$NO_RECORD" -eq 1 ]; then
        jq -c . <<<"$record"
    else
        size_append_record "$LEDGER" "$record"
    fi
}

for package in "${PACKAGES[@]}"; do
    if [ ! -f "$ROOT/$package/Cargo.toml" ]; then
        printf 'size: missing: %s (package-not-in-workspace)\n' "$package" >&2
        record_missing "$package" "package-not-in-workspace" "$package"
        MISSING_COUNT=$((MISSING_COUNT + 1))
        continue
    fi

    load_metadata
    package_exists=$(jq -r --arg package "$package" \
        'any(.packages[]; .name == $package)' <<<"$METADATA")
    if [ "$package_exists" != "true" ]; then
        printf 'size: missing: %s (not-a-workspace-member)\n' "$package" >&2
        record_missing "$package" "not-a-workspace-member" "$package"
        MISSING_COUNT=$((MISSING_COUNT + 1))
        continue
    fi

    bins=$(jq -r --arg package "$package" '
        .packages[] | select(.name == $package)
        | .targets[] | select(.kind | index("bin"))
        | .name
    ' <<<"$METADATA")
    bin_count=$(grep -c . <<<"$bins" || true)
    if [ "$bin_count" -eq 0 ]; then
        printf 'size: missing: %s (no-bin-target)\n' "$package" >&2
        record_missing "$package" "no-bin-target" "$package"
        MISSING_COUNT=$((MISSING_COUNT + 1))
        continue
    fi
    if [ "$bin_count" -gt 1 ]; then
        size_die "package $package has more than one binary target: $(tr '\n' ' ' <<<"$bins")"
    fi
    bin=$(head -n 1 <<<"$bins")

    measure_package "$package" "$bin"
done

if [ "$DRY_RUN" -eq 1 ]; then
    printf 'size: dry run complete: %d requested package(s), %d missing\n' \
        "${#PACKAGES[@]}" "$MISSING_COUNT"
    if [ "$MISSING_COUNT" -gt 0 ] && [ "$ALLOW_MISSING" -ne 1 ]; then
        exit 3
    fi
    exit 0
fi

printf 'size: measured %d package(s), missing %d\n' "$MEASURED_COUNT" "$MISSING_COUNT" >&2
if [ "$MISSING_COUNT" -gt 0 ] && [ "$ALLOW_MISSING" -ne 1 ]; then
    exit 3
fi
