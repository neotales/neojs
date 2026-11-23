#!/usr/bin/env bash
# Deterministic tests for the size tracker. No build, no network and no
# mutation of the committed ledger: fixtures live in a temporary directory.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

size_require jq sha256sum bash git mktemp

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
      schema: 1,
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
        git_describe: "c68cab5",
        git_tags: [],
        kind: "commit",
        measured_from: "worktree"
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
        features: [],
        no_default_features: false,
        offline: true,
        locked: true,
        command: "cargo +1.100.0 build --release --locked --offline --target x86_64-unknown-linux-gnu -p neojs",
        cargo_lock_sha256: ("a" * 64),
        workspace_path_sha256: ("b" * 64),
        v8_archive: null,
        env_overrides: {}
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

add_keys() {
    local record=${1-} key context_key
    if [ -z "$record" ]; then
        record=$(cat)
    fi
    key=$(size_record_key "$record")
    context_key=$(size_context_key "$record")
    jq -c --arg key "$key" --arg context_key "$context_key" \
        '.key = $key | .context_key = $context_key' <<<"$record"
}

rewrite_with_keys() {
    local filter=$1 record=$2
    add_keys "$(jq -c "$filter" <<<"$record")"
}

# Shell syntax is cheap to check and catches a typo before a measurement run.
for script in "$SCRIPT_DIR"/*.sh; do
    if bash -n "$script"; then
        ok "syntax: $(basename "$script")"
    else
        fail "syntax: $(basename "$script")"
    fi
done

VALID=$(add_keys "$(fixture)")
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

printf '%s\n' "$(rewrite_with_keys '.schema = 2' "$VALID")" >"$TMP/bad-schema.jsonl"
expect_validation_failure "wrong schema rejected" "$TMP/bad-schema.jsonl"

printf '%s\n' "$(rewrite_with_keys '.measurement.bytes = 0' "$VALID")" >"$TMP/zero.jsonl"
expect_validation_failure "zero bytes rejected" "$TMP/zero.jsonl"

printf '%s\n' "$(rewrite_with_keys '.measurement.sha256 = "not-a-sha"' "$VALID")" >"$TMP/bad-sha.jsonl"
expect_validation_failure "malformed sha256 rejected" "$TMP/bad-sha.jsonl"

printf '%s\n' "$(rewrite_with_keys '.artifact.path = "/tmp/neojs"' "$VALID")" >"$TMP/abs-path.jsonl"
expect_validation_failure "absolute artifact path rejected" "$TMP/abs-path.jsonl"

printf '%s\n' "$(jq -c '.key = ("0" * 64)' <<<"$VALID")" >"$TMP/bad-key.jsonl"
expect_validation_failure "hand-edited key rejected" "$TMP/bad-key.jsonl"

printf '%s\n%s\n' "$VALID" "$VALID" >"$TMP/duplicate.jsonl"
expect_validation_failure "duplicate key rejected" "$TMP/duplicate.jsonl"

MISSING=$(rewrite_with_keys '
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

printf '%s\n' "$(rewrite_with_keys '.measurement.bytes = 1' "$MISSING")" >"$TMP/missing-bytes.jsonl"
expect_validation_failure "missing record with bytes rejected" "$TMP/missing-bytes.jsonl"

DRIFT=$(rewrite_with_keys '
  .measurement.bytes = 12345679
  | .measurement.sha256 = ("d" * 64)
' "$VALID")
printf '%s\n%s\n' "$VALID" "$DRIFT" >"$TMP/drift.jsonl"
expect_validation_failure "same context with a different digest rejected" "$TMP/drift.jsonl"

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
if jq -e '.key and .context_key' "$TMP/raw.jsonl" >/dev/null 2>&1; then
    ok "append derives keys for a record that lacks them"
else
    fail "append did not derive keys"
    cat "$TMP/raw.err" >&2
fi

SECOND=$(jq -c '
  .artifact.package = "neojs-lite"
  | .artifact.bin = "neojs-lite"
  | .artifact.path = "neojs-lite"
' <<<"$VALID" | add_keys)
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

DRIFT_LEDGER="$TMP/drift-append.jsonl"
size_append_record "$DRIFT_LEDGER" "$VALID" >/dev/null 2>&1
size_append_record "$DRIFT_LEDGER" "$DRIFT" >"$TMP/drift.log" 2>&1
assert_contains "$TMP/drift.log" "not reproducing itself" "non-reproducible build is warned about"

expect_status 0 "dry run succeeds" \
    bash "$SCRIPT_DIR/measure.sh" --dry-run --package neojs --ledger "$TMP/dry.jsonl"
assert_contains "$TMP/stdout" "--release" "dry run prints the release command"
assert_contains "$TMP/stdout" "dry run complete" "dry run reports completion"

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

expect_status 0 "report renders a valid ledger" \
    bash "$SCRIPT_DIR/report.sh" --ledger "$TMP/valid.jsonl"
assert_contains "$TMP/stdout" "neojs" "report includes the package"
assert_contains "$TMP/stdout" "12345678" "report includes the byte count"

if [ "$FAILED" -ne 0 ]; then
    printf 'size: test failures\n' >&2
    exit 1
fi
printf 'size: all deterministic checks passed\n'
