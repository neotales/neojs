#!/usr/bin/env bash
# Deterministic checks for the bounded V8 build wrapper.
#
# This does not run a V8 build. It checks that two runs with the same clock
# second still get distinct `mktemp -d` run roots, that the clean procedure
# helper set is accepted, and that ignored, untracked, modified, or
# mode-changed helpers are rejected.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/neojs-build-bounded-test.XXXXXX")
mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/cache" "$TEST_ROOT/procedure" "$TEST_ROOT/build"
cat > "$TEST_ROOT/bin/date" <<'SH'
#!/bin/sh
printf '20260101T000000Z\n'
SH
chmod +x "$TEST_ROOT/bin/date"

run_root() {
    PATH="$TEST_ROOT/bin:$PATH" \
    NEOTALES_V8_CACHE="$TEST_ROOT/cache" \
    NEOTALES_V8_PROCEDURE="$TEST_ROOT/procedure" \
    NEOJS_V8_BUILD_DIR="$TEST_ROOT/build" \
    "$SCRIPT_DIR/build-bounded.sh" --run-root-only
}

first=$(run_root | sed -n 's/^run_root //p')
second=$(run_root | sed -n 's/^run_root //p')
[ -n "$first" ] && [ -n "$second" ] || {
    echo "error: run-root-only did not print a root" >&2
    exit 1
}
[ "$first" != "$second" ] || {
    echo "error: run root collision: $first" >&2
    exit 1
}
[ -d "$first" ] && [ -d "$second" ] || {
    echo "error: a run root was not created" >&2
    exit 1
}

make_procedure() {
    local dir=$1
    mkdir -p "$dir/eng/v8-source"
    printf '#!/bin/sh\nexit 0\n' > "$dir/eng/v8-source/prepare.sh"
    chmod +x "$dir/eng/v8-source/prepare.sh"
    for helper in validate_tar.py verify_git_tree.py materialize_git_tree.py; do
        printf '# %s\n' "$helper" > "$dir/eng/v8-source/$helper"
        chmod 644 "$dir/eng/v8-source/$helper"
    done
    git -C "$dir" init -q
    git -C "$dir" add .
    git -C "$dir" -c user.name=test -c user.email=test@example.invalid commit -qm initial
}

check_procedure() {
    local proc=$1
    PATH="$TEST_ROOT/bin:$PATH" \
    NEOTALES_V8_CACHE="$TEST_ROOT/cache" \
    NEOTALES_V8_PROCEDURE="$proc" \
    NEOJS_V8_BUILD_DIR="$TEST_ROOT/build" \
    "$SCRIPT_DIR/build-bounded.sh" --check-procedure-only
}

expect_failure() {
    local name=$1 proc=$2 pattern=$3
    if check_procedure "$proc" >"$TEST_ROOT/$name.log" 2>&1; then
        echo "error: $name was accepted" >&2
        exit 1
    fi
    grep -q "$pattern" "$TEST_ROOT/$name.log" || {
        echo "error: $name did not report '$pattern'" >&2
        cat "$TEST_ROOT/$name.log" >&2
        exit 1
    }
}

CLEAN="$TEST_ROOT/clean-procedure"
make_procedure "$CLEAN"
clean_output=$(check_procedure "$CLEAN")
printf '%s\n' "$clean_output" |
    grep -q '^procedure [0-9a-f]* helper-tree [0-9a-f]* files 4$' || {
    echo "error: clean procedure was not verified: $clean_output" >&2
    exit 1
}

IGNORED="$TEST_ROOT/ignored-procedure"
make_procedure "$IGNORED"
printf 'eng/v8-source/ignored_helper.py\n' > "$IGNORED/.gitignore"
git -C "$IGNORED" add .gitignore
git -C "$IGNORED" -c user.name=test -c user.email=test@example.invalid commit -qm ignore
printf '# ignored\n' > "$IGNORED/eng/v8-source/ignored_helper.py"
expect_failure ignored "$IGNORED" "untracked, ignored"

UNTRACKED="$TEST_ROOT/untracked-procedure"
make_procedure "$UNTRACKED"
printf '# untracked\n' > "$UNTRACKED/eng/v8-source/untracked_helper.py"
expect_failure untracked "$UNTRACKED" "must be clean"

MODIFIED="$TEST_ROOT/modified-procedure"
make_procedure "$MODIFIED"
printf '# modified\n' >> "$MODIFIED/eng/v8-source/validate_tar.py"
expect_failure modified "$MODIFIED" "must be clean"

MODE="$TEST_ROOT/mode-procedure"
make_procedure "$MODE"
git -C "$MODE" config core.filemode false
chmod +x "$MODE/eng/v8-source/validate_tar.py"
expect_failure mode "$MODE" "unexpectedly executable"

printf 'ok: unique run roots, clean helper verification, and helper rejection cases\n'
printf 'test root: %s\n' "$TEST_ROOT"
