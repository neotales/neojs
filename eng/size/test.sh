#!/usr/bin/env bash
# Deterministic tests for the size tracker. No build, no network and no
# mutation of the committed ledger: fixtures live in a temporary directory.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

size_require jq sha256sum bash git mktemp flock gzip cargo

ROOT=$(size_repo_root)
mkdir -p "$ROOT/target/size/tmp"
TMP=$(mktemp -d "$ROOT/target/size/tmp/test.XXXXXX")
export TMPDIR="$ROOT/target/size/tmp"
trap 'rm -rf -- "$TMP"' EXIT
FAILED=0

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    FAILED=1
}

ok() {
    printf 'ok: %s\n' "$*"
}

assert_contains() {
    local file=$1 needle=$2 label=$3
    if grep -Fq -- "$needle" "$file"; then
        ok "$label"
    else
        fail "$label (missing '$needle')"
    fi
}

expect_status() {
    local expected=$1 label=$2
    shift 2
    local status
    set +e
    "$@" >"$TMP/stdout" 2>"$TMP/stderr"
    status=$?
    set -e
    if [ "$status" -eq "$expected" ]; then
        ok "$label (exit $status)"
    else
        fail "$label (expected exit $expected, got $status)"
        sed -n '1,20p' "$TMP/stderr" >&2
    fi
}

expect_validation_failure() {
    local label=$1 ledger=$2
    if (size_validate_ledger "$ledger" 0) >"$TMP/stdout" 2>"$TMP/stderr"; then
        fail "$label (validator accepted the ledger)"
    else
        ok "$label (validator rejected the ledger)"
    fi
}

fixture() {
    jq -n '{
      schema: 2,
      status: "ok",
      artifact: {
        package: "neojs",
        bin: "neojs",
        kind: "release-binary",
        path: "neojs"
      },
      source: {
        git_commit: "c68cab512999683fbc14bca557a6647a307f55ac",
        git_branch: "dev",
        git_dirty: false,
        git_canonical: true,
        git_describe: "c68cab5",
        git_tags: [],
        kind: "commit",
        measured_from: "worktree",
        state_status: "captured",
        state_sha256: ("e" * 64)
      },
      platform: {
        target: "x86_64-unknown-linux-gnu",
        host: "x86_64-unknown-linux-gnu",
        os: "Linux",
        arch: "x86_64",
        kernel: "7.2.15"
      },
      toolchain: {
        requested: "1.100.0",
        rustc: "rustc 1.100.0 (0ef1bc386 2026-11-09)",
        cargo: "cargo 1.100.0 (3acc79c0c 2026-11-05)",
        llvm: "23.1.5",
        cc: "cc (GCC) 15.2.1",
        cxx: "c++ (GCC) 15.2.1"
      },
      build: {
        profile: "release",
        requested_features: [],
        requested_no_default_features: false,
        effective_features: {status: "not-captured"},
        effective_linker: {status: "not-captured"},
        offline: true,
        locked: true,
        command: "cargo +1.100.0 build --release --locked --offline --target x86_64-unknown-linux-gnu -p neojs",
        cargo_lock_sha256: ("a" * 64),
        workspace_path_sha256: ("b" * 64),
        cargo_config: {status: "captured", files: [{scope: "workspace", sha256: ("f" * 64)}]},
        env_overrides: {},
        target_specific_env: {status: "absent", names: []},
        v8_archive_status: "not-applicable",
        v8_archive: null
      },
      measurement: {
        method: "file-length-bytes",
        bytes: 12345678,
        sha256: ("c" * 64),
        file: "ELF 64-bit LSB pie executable"
      },
      missing_reason: null,
      measured_at: "2026-11-22T00:00:00Z",
      notes: null
    }'
}

finalize() {
    local record=${1-}
    if [ -z "$record" ]; then
        record=$(cat)
    fi
    size_finalize_record "$record"
}

rewrite_with_ids() {
    local filter=$1 record=$2
    finalize "$(jq -c "$filter" <<<"$record")"
}

make_workspace() {
    local root=$1 package=$2
    mkdir -p "$root/eng" "$root/$package/src"
    cp -a "$SCRIPT_DIR" "$root/eng/size"
    cat >"$root/Cargo.toml" <<TOML
[workspace]
resolver = "3"
members = ["$package"]
TOML
    cat >"$root/$package/Cargo.toml" <<TOML
[package]
name = "$package"
version = "0.1.0"
edition = "2024"
TOML
    printf 'fn main() {}\n' >"$root/$package/src/main.rs"
    printf '/target/\n' >"$root/.gitignore"
    cargo generate-lockfile --manifest-path "$root/Cargo.toml" --offline >/dev/null 2>&1
    git -c init.defaultBranch=dev init -q "$root"
    git -C "$root" add .
    git -C "$root" -c user.name=Test -c user.email=test@example.com commit -q -m base
}

# Shell syntax is cheap to check and catches a typo before a measurement run.
for script in "$SCRIPT_DIR"/*.sh; do
    if bash -n "$script"; then
        ok "syntax: $(basename "$script")"
    else
        fail "syntax: $(basename "$script")"
    fi
done

VALID=$(finalize "$(fixture)")
printf '%s\n' "$VALID" >"$TMP/valid.jsonl"
if (size_validate_ledger "$TMP/valid.jsonl" 0) >"$TMP/stdout" 2>"$TMP/stderr"; then
    ok "valid record accepted"
else
    fail "valid record rejected"
    cat "$TMP/stderr" >&2
fi

: >"$TMP/empty.jsonl"
expect_validation_failure "empty ledger rejected" "$TMP/empty.jsonl"
if (size_validate_ledger "$TMP/empty.jsonl" 1) >/dev/null 2>&1; then
    ok "empty ledger accepted with allow-empty"
else
    fail "empty ledger rejected with allow-empty"
fi

printf '{\n' >"$TMP/bad-json.jsonl"
expect_validation_failure "invalid JSON rejected" "$TMP/bad-json.jsonl"

printf '%s\n' "$(rewrite_with_ids '.schema = 1' "$VALID")" >"$TMP/bad-schema.jsonl"
expect_validation_failure "wrong schema rejected" "$TMP/bad-schema.jsonl"

printf '%s\n' "$(rewrite_with_ids '.measurement.bytes = 0' "$VALID")" >"$TMP/zero.jsonl"
expect_validation_failure "zero bytes rejected" "$TMP/zero.jsonl"

printf '%s\n' "$(rewrite_with_ids '.measurement.sha256 = "not-a-sha"' "$VALID")" >"$TMP/bad-sha.jsonl"
expect_validation_failure "malformed sha256 rejected" "$TMP/bad-sha.jsonl"

printf '%s\n' "$(rewrite_with_ids '.artifact.path = "/tmp/neojs"' "$VALID")" >"$TMP/abs-path.jsonl"
expect_validation_failure "absolute artifact path rejected" "$TMP/abs-path.jsonl"

printf '%s\n' "$(rewrite_with_ids '.build.features = []' "$VALID")" >"$TMP/legacy-features.jsonl"
expect_validation_failure "legacy features field rejected" "$TMP/legacy-features.jsonl"

printf '%s\n' "$(rewrite_with_ids '.source.git_canonical = false' "$VALID")" >"$TMP/bad-canonical.jsonl"
expect_validation_failure "canonical must match dirty state" "$TMP/bad-canonical.jsonl"

printf '%s\n' "$(rewrite_with_ids '.source.state_sha256 = null' "$VALID")" >"$TMP/bad-state.jsonl"
expect_validation_failure "captured state needs a digest" "$TMP/bad-state.jsonl"

printf '%s\n' "$(rewrite_with_ids '.build.cargo_config = {status: "not-captured", files: [{scope: "workspace", sha256: ("f" * 64)}]}' "$VALID")" >"$TMP/bad-config.jsonl"
expect_validation_failure "not-captured config cannot list files" "$TMP/bad-config.jsonl"

V8=$(rewrite_with_ids '
  .build.v8_archive_status = "supplied-provenance-unknown"
  | .build.v8_archive = {
      name: "librusty_v8.a.gz",
      sha256: ("d" * 64),
      archive_sha256: ("e" * 64),
      provenance: "operator-supplied-local-file",
      authenticated: false
    }
' "$VALID")
printf '%s\n' "$V8" >"$TMP/v8.jsonl"
if (size_validate_ledger "$TMP/v8.jsonl" 0) >/dev/null 2>&1; then
    ok "unauthenticated V8 provenance accepted with explicit labels"
else
    fail "unauthenticated V8 provenance record rejected"
fi
printf '%s\n' "$(rewrite_with_ids '.build.v8_archive.authenticated = true' "$V8")" >"$TMP/v8-auth.jsonl"
expect_validation_failure "authenticated V8 archive claim rejected" "$TMP/v8-auth.jsonl"

printf '%s\n' "$(rewrite_with_ids '.build.command = "/home/dev/neojs/target/release/neojs"' "$VALID")" >"$TMP/machine-command.jsonl"
expect_validation_failure "machine path in command rejected" "$TMP/machine-command.jsonl"

printf '%s\n' "$(jq -c '.key = ("0" * 64)' <<<"$VALID")" >"$TMP/bad-key.jsonl"
expect_validation_failure "hand-edited key rejected" "$TMP/bad-key.jsonl"

printf '%s\n%s\n' "$VALID" "$VALID" >"$TMP/duplicate.jsonl"
expect_validation_failure "duplicate key rejected" "$TMP/duplicate.jsonl"

MISSING=$(rewrite_with_ids '
  .status = "missing"
  | .missing_reason = "package-not-in-workspace"
  | .build.command = ""
  | .measurement = {method: null, bytes: null, sha256: null, file: null}
' "$VALID")
printf '%s\n' "$MISSING" >"$TMP/missing.jsonl"
if (size_validate_ledger "$TMP/missing.jsonl" 0) >/dev/null 2>"$TMP/stderr"; then
    ok "missing record accepted"
else
    fail "missing record rejected"
    cat "$TMP/stderr" >&2
fi

printf '%s\n' "$(rewrite_with_ids '.measurement.bytes = 1' "$MISSING")" >"$TMP/missing-bytes.jsonl"
expect_validation_failure "missing record with bytes rejected" "$TMP/missing-bytes.jsonl"

DRIFT=$(rewrite_with_ids '
  .measurement.bytes = 12345679
  | .measurement.sha256 = ("d" * 64)
' "$VALID")
printf '%s\n%s\n' "$VALID" "$DRIFT" >"$TMP/drift.jsonl"
expect_validation_failure "same series and state with a different digest rejected under strict validation" "$TMP/drift.jsonl"
if (size_validate_ledger "$TMP/drift.jsonl" 0 allow-conflicts) >/dev/null 2>&1; then
    ok "conflicts are warnings when requested"
else
    fail "allow-conflicts rejected a conflicting ledger"
fi
expect_status 1 "validate fails a conflict by default" \
    bash "$SCRIPT_DIR/validate.sh" --ledger "$TMP/drift.jsonl"
expect_status 0 "validate allows a conflict when asked" \
    bash "$SCRIPT_DIR/validate.sh" --ledger "$TMP/drift.jsonl" --allow-conflicts
expect_status 0 "report still renders a conflicting ledger" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/drift.jsonl"
assert_contains "$TMP/stdout" "conflict" "report names the conflict column"
assert_contains "$TMP/stdout" "yes" "report marks the conflicted series"

TOOL2=$(rewrite_with_ids '
  .toolchain.requested = "1.101.0"
  | .toolchain.rustc = "rustc 1.101.0 (111111111 2026-12-01)"
  | .measurement.bytes = 12340000
  | .measurement.sha256 = ("d" * 64)
  | .measured_at = "2026-11-23T00:00:00Z"
' "$VALID")
printf '%s\n%s\n' "$VALID" "$TOOL2" >"$TMP/trend.jsonl"
if (size_validate_ledger "$TMP/trend.jsonl" 0 strict) >/dev/null 2>&1; then
    ok "toolchain change is a separate strict series, not a conflict"
else
    fail "toolchain change was rejected as a conflict"
fi
expect_status 0 "trend report renders" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/trend.jsonl" --trend
assert_contains "$TMP/stdout" "controls-differ" "trend labels a controls-differ delta"
assert_contains "$TMP/stdout" "toolchain" "trend names the differing control"
assert_contains "$TMP/stdout" "-5678" "trend shows a signed delta"

UNCAPPED=$(rewrite_with_ids '
  .build.cargo_config = {status: "not-captured", files: []}
  | .build.target_specific_env = {status: "not-captured", names: []}
  | .source.state_status = "not-captured"
  | .source.state_sha256 = null
  | .measured_at = "2026-11-23T01:00:00Z"
' "$VALID")
printf '%s\n%s\n' "$UNCAPPED" "$VALID" >"$TMP/capture-trend.jsonl"
expect_status 0 "trend labels capture-status changes" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/capture-trend.jsonl" --trend
assert_contains "$TMP/stdout" "cargo-config-capture" "trend distinguishes unknown-to-captured config"
assert_contains "$TMP/stdout" "source-state-capture" "trend distinguishes unknown-to-captured source state"

DIRTYVALID=$(rewrite_with_ids '
  .source.git_dirty = true
  | .source.git_canonical = false
  | .source.state_sha256 = ("9" * 64)
  | .measured_at = "2026-11-23T02:00:00Z"
' "$VALID")
printf '%s\n%s\n' "$VALID" "$DIRTYVALID" >"$TMP/dirty-trend.jsonl"
expect_status 0 "trend labels source-state deltas" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/dirty-trend.jsonl" --trend
assert_contains "$TMP/stdout" "source-state" "trend labels a non-canonical source delta"
assert_contains "$TMP/stdout" "source-canonical" "trend names the canonical change"


: >"$TMP/append.jsonl"
size_append_record "$TMP/append.jsonl" "$VALID" >"$TMP/append.log" 2>&1
size_append_record "$TMP/append.jsonl" "$VALID" >>"$TMP/append.log" 2>&1
if [ "$(wc -l <"$TMP/append.jsonl" | tr -d '[:space:]')" -eq 1 ]; then
    ok "second identical append is a no-op"
else
    fail "second identical append duplicated the record"
fi
assert_contains "$TMP/append.log" "already recorded" "idempotent append reports the existing key"

RAW=$(fixture)
size_append_record "$TMP/raw.jsonl" "$RAW" >/dev/null 2>"$TMP/raw.err"
if jq -e '.key and .series_id and .family_id' "$TMP/raw.jsonl" >/dev/null 2>&1; then
    ok "append derives keys for a record that lacks them"
else
    fail "append did not derive keys"
    cat "$TMP/raw.err" >&2
fi

printf '{"schema":2}\n' >"$TMP/bad-append.jsonl"
if (size_append_record "$TMP/bad-append.jsonl" "$RAW") >/dev/null 2>&1; then
    fail "append refuses an already-invalid ledger"
else
    ok "append refuses an already-invalid ledger"
fi

SECOND=$(jq -c '
  .artifact.package = "neojs-lite"
  | .artifact.bin = "neojs-lite"
  | .artifact.path = "neojs-lite"
' <<<"$VALID" | finalize)
size_append_record "$TMP/append.jsonl" "$SECOND" >/dev/null 2>&1
if [ "$(wc -l <"$TMP/append.jsonl" | tr -d '[:space:]')" -eq 2 ]; then
    ok "a different artifact appends"
else
    fail "a different artifact did not append"
fi

REPO="$TMP/repo"
mkdir -p "$REPO"
git -c init.defaultBranch=dev init -q "$REPO"
printf 'base\n' >"$REPO/ledger.jsonl"
git -C "$REPO" add ledger.jsonl
git -C "$REPO" -c user.name=Test -c user.email=test@example.com commit -q -m base
printf '%s\n' "$VALID" >>"$REPO/ledger.jsonl"
if size_git_dirty "$REPO" "$REPO/ledger.jsonl"; then
    fail "ledger changes are excluded from the dirty check"
else
    ok "ledger changes are excluded from the dirty check"
fi
printf 'change\n' >"$REPO/source.txt"
if size_git_dirty "$REPO" "$REPO/ledger.jsonl"; then
    ok "source changes are included in the dirty check"
else
    fail "source changes are excluded from the dirty check"
fi

printf 'one\n' >"$REPO/source.txt"
STATE_ONE=$(size_git_state_sha256 "$REPO" "$REPO/ledger.jsonl")
printf 'two\n' >"$REPO/source.txt"
STATE_TWO=$(size_git_state_sha256 "$REPO" "$REPO/ledger.jsonl")
if [ "$STATE_ONE" != "$STATE_TWO" ]; then
    ok "state fingerprint detects content changes behind the same dirty status"
else
    fail "state fingerprint ignored a content change"
fi

# The build lock must exclude a second process for the same target directory.
( size_lock_target_dir "$TMP/build.lock"; : >"$TMP/lock-held"; sleep 1 ) &
LOCK_PID=$!
for _ in $(seq 1 100); do
    [ -f "$TMP/lock-held" ] && break
    sleep 0.02
done
if flock -n "$TMP/build.lock" true 2>/dev/null; then
    fail "target-directory lock did not exclude a second holder"
else
    ok "target-directory lock excludes a second holder"
fi
wait "$LOCK_PID"
if flock -n "$TMP/build.lock" true 2>/dev/null; then
    ok "target-directory lock is released"
else
    fail "target-directory lock stayed held after release"
fi

# A relative --target-dir is relative to the caller, then normalized.
# shellcheck disable=SC2016 # the $1/$2/$3 are arguments to the inner bash
expect_status 0 "relative target-dir is normalized" \
    bash -c 'cd "$1" && exec bash "$2" --dry-run --package neojs --target-dir relbuild --ledger "$3"' \
    _ "$TMP" "$SCRIPT_DIR/measure.sh" "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "$TMP/relbuild/" "relative target-dir resolved against the caller"

expect_status 1 "RUSTFLAGS is refused" \
    env RUSTFLAGS=-Copt-level=0 bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs-lite
assert_contains "$TMP/stderr" "RUSTFLAGS" "RUSTFLAGS refusal names the variable"
expect_status 0 "RUSTFLAGS is recorded when explicitly allowed" \
    env RUSTFLAGS=-Copt-level=0 bash "$SCRIPT_DIR/measure.sh" --dry-run --allow-env-overrides --package neojs-lite
expect_status 1 "target-specific env is refused" \
    env CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Copt-level=0 \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs-lite
assert_contains "$TMP/stderr" "CARGO_TARGET_" "target-specific refusal names the variable"
expect_status 0 "target-specific env is recorded when explicitly allowed" \
    env CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Copt-level=0 \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --allow-env-overrides --package neojs-lite

DRIFT_LEDGER="$TMP/drift-append.jsonl"
size_append_record "$DRIFT_LEDGER" "$VALID" >/dev/null 2>&1
size_append_record "$DRIFT_LEDGER" "$DRIFT" >"$TMP/drift.log" 2>&1
assert_contains "$TMP/drift.log" "same strict series" "non-reproducible build is warned about"

expect_status 0 "dry run succeeds" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs --ledger "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "--release" "dry run prints the release command"
assert_contains "$TMP/stdout" "dry run complete" "dry run reports completion"
if grep -Fq -- "--manifest-path" "$TMP/stdout"; then
    fail "recorded build command contains a machine-specific manifest path"
else
    ok "recorded build command has no machine-specific manifest path"
fi

expect_status 0 "dry run honours an explicit target directory" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs --target-dir "$TMP/size-build" \
    --ledger "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "$TMP/size-build/" "dry run artifact path uses the dedicated target directory"
if [ ! -e "$TMP/dry.jsonl" ]; then
    ok "dry run does not create a ledger"
else
    fail "dry run created a ledger"
fi

printf 'fake-v8\n' | gzip -c >"$TMP/fake-v8.a.gz"
expect_status 0 "dry run attributes the V8 archive to neojs" \
    env RUSTY_V8_ARCHIVE="$TMP/fake-v8.a.gz" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs --ledger "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "v8 archive input: {" "neojs records the V8 archive"
assert_contains "$TMP/stdout" '"archive_sha256"' "neojs records the wrapper hash for provenance"

expect_status 0 "dry run omits the V8 archive for neojs-lite" \
    env RUSTY_V8_ARCHIVE="$TMP/fake-v8.a.gz" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs-lite --ledger "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "v8 archive input: null" "neojs-lite does not record an unused V8 archive"

expect_status 3 "missing package exits 3" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package not-a-package --ledger "$TMP/missing-dry.jsonl"
assert_contains "$TMP/stderr" "package-not-in-workspace" "missing package names the reason"

expect_status 0 "allow-missing accepts a missing package" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --allow-missing --package not-a-package --ledger "$TMP/missing-dry.jsonl"

# A real workspace with no neojs-lite member exercises the branch that a branch
# without Lite would take, without touching this checkout.
FAKE="$TMP/fakeroot"
mkdir -p "$FAKE/eng" "$FAKE/neojs/src"
cp -a "$SCRIPT_DIR" "$FAKE/eng/size"
cat >"$FAKE/Cargo.toml" <<'TOML'
[workspace]
resolver = "3"
members = ["neojs"]
TOML
cat >"$FAKE/neojs/Cargo.toml" <<'TOML'
[package]
name = "neojs"
version = "0.1.0"
edition = "2024"
TOML
printf 'fn main() {}\n' >"$FAKE/neojs/src/main.rs"
cargo generate-lockfile --manifest-path "$FAKE/Cargo.toml" --offline >/dev/null 2>&1
expect_status 3 "fixture workspace without Lite exits 3" \
    bash "$FAKE/eng/size/measure.sh" --dry-run --package neojs --package neojs-lite \
    --ledger "$TMP/fake-missing.jsonl"
assert_contains "$TMP/stderr" "neojs-lite" "fixture missing report names neojs-lite"
assert_contains "$TMP/stderr" "package-not-in-workspace" "fixture missing report names the reason"
assert_contains "$TMP/stdout" "dry run: neojs" "fixture still measures the present package"

DIRTYROOT="$TMP/dirtyroot"
make_workspace "$DIRTYROOT" neojs
printf '// dirty\n' >>"$DIRTYROOT/neojs/src/main.rs"
expect_status 1 "dirty canonical measurement is refused" \
    bash "$DIRTYROOT/eng/size/measure.sh" --no-record --package neojs \
    --target-dir "$TMP/dirty-build" --ledger "$TMP/dirty-ledger.jsonl"
assert_contains "$TMP/stderr" "source tree is dirty" "dirty refusal explains the state"
expect_status 0 "dirty measurement is recorded with --allow-dirty" \
    bash "$DIRTYROOT/eng/size/measure.sh" --no-record --allow-dirty --package neojs \
    --target-dir "$TMP/dirty-build" --ledger "$TMP/dirty-ledger.jsonl"
if jq -e '.source.git_dirty == true and .source.git_canonical == false and .source.state_status == "captured"' \
    "$TMP/stdout" >/dev/null 2>&1; then
    ok "dirty record is explicitly non-canonical and state-fingerprinted"
else
    fail "dirty record lacked canonical/state labels"
fi

CHANGEROOT="$TMP/changeroot"
make_workspace "$CHANGEROOT" neojs
cat >"$CHANGEROOT/neojs/build.rs" <<'RS'
fn main() {
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../generated.txt"),
        "generated\n",
    )
    .unwrap();
}
RS
git -C "$CHANGEROOT" add neojs/build.rs
git -C "$CHANGEROOT" -c user.name=Test -c user.email=test@example.com commit -q -m build-script
expect_status 1 "source state change during build is refused" \
    bash "$CHANGEROOT/eng/size/measure.sh" --no-record --package neojs \
    --target-dir "$TMP/change-build" --ledger "$TMP/change-ledger.jsonl"
assert_contains "$TMP/stderr" "changed while building" "state-change refusal explains the reason"

CONFIGROOT="$TMP/configroot"
make_workspace "$CONFIGROOT" neojs
mkdir -p "$CONFIGROOT/.cargo"
printf '[build]\njobs = 1\n' >"$CONFIGROOT/.cargo/config.toml"
git -C "$CONFIGROOT" add .cargo/config.toml
git -C "$CONFIGROOT" -c user.name=Test -c user.email=test@example.com commit -q -m cargo-config
CONFIG_SHA=$(sha256sum "$CONFIGROOT/.cargo/config.toml" | awk '{print $1}')
expect_status 0 "workspace Cargo config is captured" \
    bash "$CONFIGROOT/eng/size/measure.sh" --no-record --package neojs \
    --target-dir "$TMP/config-build" --ledger "$TMP/config-ledger.jsonl"
if jq -e --arg sha "$CONFIG_SHA" \
    'any(.build.cargo_config.files[]; .scope == "workspace" and .sha256 == $sha)' \
    "$TMP/stdout" >/dev/null 2>&1; then
    ok "workspace Cargo config hash is recorded"
else
    fail "workspace Cargo config hash was not recorded"
fi

expect_status 0 "report renders a valid ledger" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/valid.jsonl"
assert_contains "$TMP/stdout" "neojs" "report includes the package"
assert_contains "$TMP/stdout" "12345678" "report includes the byte count"

if [ "$FAILED" -ne 0 ]; then
    printf 'size: test failures\n' >&2
    exit 1
fi
printf 'size: all deterministic checks passed\n'
