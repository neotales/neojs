#!/usr/bin/env bash
# Deterministic checks for the bounded V8 build wrapper.
#
# This does not run a V8 build. It checks that two runs with the same clock
# second still get distinct `mktemp -d` run roots, and that a dirty procedure
# worktree is rejected before any source is copied.
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

DIRTY="$TEST_ROOT/dirty-procedure"
mkdir -p "$DIRTY/eng/v8-source"
printf '#!/bin/sh\nexit 0\n' > "$DIRTY/eng/v8-source/prepare.sh"
chmod +x "$DIRTY/eng/v8-source/prepare.sh"
git -C "$DIRTY" init -q
git -C "$DIRTY" add .
git -C "$DIRTY" -c user.name=test -c user.email=test@example.invalid commit -qm initial
printf '\n# dirty\n' >> "$DIRTY/eng/v8-source/prepare.sh"

if PATH="$TEST_ROOT/bin:$PATH" \
    NEOTALES_V8_CACHE="$TEST_ROOT/cache" \
    NEOTALES_V8_PROCEDURE="$DIRTY" \
    NEOJS_V8_BUILD_DIR="$TEST_ROOT/build" \
    "$SCRIPT_DIR/build-bounded.sh" --prepare-only >"$TEST_ROOT/dirty.log" 2>&1; then
    echo "error: a dirty procedure worktree was accepted" >&2
    exit 1
fi
grep -q "must be clean for provenance" "$TEST_ROOT/dirty.log" || {
    cat "$TEST_ROOT/dirty.log" >&2
    exit 1
}

printf 'ok: unique run roots and dirty-procedure rejection\n'
printf 'test root: %s\n' "$TEST_ROOT"
