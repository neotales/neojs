#!/usr/bin/env bash
# Deterministic tests for eng/v8-source/prepare.sh and validate_tar.py.
#
# No network and no real V8 inputs are used. Fixtures live under
# $NEOTALES_V8_TEST_CACHE, which defaults to the review cache path. The script
# never touches a production cache.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=${NEOTALES_V8_TEST_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-v8-build-tests}
RUN=$ROOT/tests/run.$(date -u +%Y%m%dT%H%M%SZ).$$
mkdir -p "$RUN"/{cache,fixtures,mockbin,out}
export LC_ALL=C

pass=0
fail=0
ok() { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
notok() { fail=$((fail + 1)); printf 'FAIL %s\n' "$1"; }

expect_fail() {
    local desc=$1
    shift
    if ("$@") >/dev/null 2>&1; then
        notok "$desc"
    else
        ok "$desc"
    fi
}

contains() {
    case $1 in
        *"$2"*) return 0 ;;
        *) return 1 ;;
    esac
}

expect_ok() {
    local desc=$1
    shift
    if ("$@") >/dev/null 2>&1; then
        ok "$desc"
    else
        notok "$desc"
    fi
}

# Load the library without its main().
# shellcheck disable=SC2034
CACHE=$RUN/cache
# shellcheck disable=SC2034
V8_PREPARE_LIB=1
# shellcheck source=/dev/null
. "$SCRIPT_DIR/prepare.sh"
init_paths
STAMPS=$RUN/cache/.stamps
STAGING=$RUN/cache/.staging
QUARANTINE=$RUN/cache/.quarantine
FAILED=$RUN/cache/.failed
TMP=$RUN/cache/.tmp
ISO_HOME=$RUN/cache/.isolated-home
mkdir -p "$STAMPS" "$STAGING" "$QUARANTINE" "$FAILED" "$TMP" "$ISO_HOME"

# --- fixtures --------------------------------------------------------------
python3 - "$RUN/fixtures" <<'PY'
import io, os, sys, tarfile

root = sys.argv[1]

def add(tf, name, data=b"x", mode=0o644, type=tarfile.REGTYPE, link=""):
    ti = tarfile.TarInfo(name)
    ti.mode = mode
    ti.type = type
    ti.linkname = link
    ti.size = len(data) if type == tarfile.REGTYPE else 0
    tf.addfile(ti, io.BytesIO(data) if type == tarfile.REGTYPE else None)

def write(name, fn):
    with tarfile.open(os.path.join(root, name), "w") as tf:
        fn(tf)

def safe(tf):
    ti = tarfile.TarInfo("pkg")
    ti.type = tarfile.DIRTYPE
    ti.mode = 0o755
    tf.addfile(ti)
    add(tf, "pkg/a.txt", b"hello\n")
    add(tf, "pkg/link", type=tarfile.SYMTYPE, link="a.txt")
    add(tf, "pkg/hard", type=tarfile.LNKTYPE, link="pkg/a.txt")

def traversal(tf):
    add(tf, "../escape", b"no")

def absolute(tf):
    add(tf, "/etc/escape", b"no")

def symlink_escape(tf):
    add(tf, "pkg/out", type=tarfile.SYMTYPE, link="../../outside")

def ancestor_symlink(tf):
    add(tf, "pkg/d", type=tarfile.SYMTYPE, link="sub")
    add(tf, "pkg/d/x", b"no")

def hardlink_escape(tf):
    add(tf, "pkg/h", type=tarfile.LNKTYPE, link="../outside")

def fifo(tf):
    ti = tarfile.TarInfo("pkg/fifo")
    ti.type = tarfile.FIFOTYPE
    tf.addfile(ti)

def setuid(tf):
    add(tf, "pkg/suid", b"x", mode=0o4755)

def duplicate(tf):
    add(tf, "pkg/dup", b"one")
    add(tf, "pkg/dup", b"two")

for name, fn in [
    ("safe.tar", safe),
    ("traversal.tar", traversal),
    ("absolute.tar", absolute),
    ("symlink_escape.tar", symlink_escape),
    ("ancestor_symlink.tar", ancestor_symlink),
    ("hardlink_escape.tar", hardlink_escape),
    ("fifo.tar", fifo),
    ("setuid.tar", setuid),
    ("duplicate.tar", duplicate),
]:
    write(name, fn)
PY

V=$SCRIPT_DIR/validate_tar.py
F=$RUN/fixtures

# --- validator -------------------------------------------------------------
expect_ok "validator accepts a safe archive" python3 "$V" --check "$F/safe.tar"
expect_fail "validator rejects ../ traversal" python3 "$V" --check "$F/traversal.tar"
expect_fail "validator rejects absolute paths" python3 "$V" --check "$F/absolute.tar"
expect_fail "validator rejects escaping symlink" python3 "$V" --check "$F/symlink_escape.tar"
expect_fail "validator rejects symlink ancestor" python3 "$V" --check "$F/ancestor_symlink.tar"
expect_fail "validator rejects escaping hardlink" python3 "$V" --check "$F/hardlink_escape.tar"
expect_fail "validator rejects fifo" python3 "$V" --check "$F/fifo.tar"
expect_fail "validator rejects setuid" python3 "$V" --check "$F/setuid.tar"
expect_fail "validator rejects duplicate member" python3 "$V" --check "$F/duplicate.tar"

# --- safe extraction -------------------------------------------------------
# shellcheck disable=SC2034
VERIFY_ONLY=0
if safe_extract "$F/safe.tar" "$RUN/out/safe"; then
    if [ "$(cat "$RUN/out/safe/pkg/a.txt")" = hello ] &&
       [ "$(readlink "$RUN/out/safe/pkg/link")" = a.txt ] &&
       [ "$(stat -c %i "$RUN/out/safe/pkg/a.txt")" = "$(stat -c %i "$RUN/out/safe/pkg/hard")" ]; then
        ok "safe_extract extracts files, relative symlink and hardlink"
    else
        notok "safe_extract content"
    fi
else
    notok "safe_extract safe archive"
fi

if (safe_extract "$F/traversal.tar" "$RUN/out/hostile") >/dev/null 2>&1; then
    notok "safe_extract refuses traversal"
else
    if [ ! -e "$RUN/out/hostile" ] && [ -n "$(find "$FAILED" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
        ok "safe_extract refuses traversal and quarantines staging"
    else
        notok "safe_extract hostile side effects"
    fi
fi

# --- tamper and partial extraction ----------------------------------------
sha=$(sha256_of "$F/safe.tar")
size=$(size_of "$F/safe.tar")
tree=$RUN/out/tree
ensure_archive_extracted "$F/safe.tar" "$tree" "$sha" "$size" "$tree/pkg"
sed -i 's/hello/HELLO/' "$tree/pkg/a.txt"
VERIFY_ONLY=1
expect_fail "verify detects tampered extracted tree" ensure_archive_extracted "$F/safe.tar" "$tree" "$sha" "$size" "$tree/pkg"
# shellcheck disable=SC2034
VERIFY_ONLY=0
ensure_archive_extracted "$F/safe.tar" "$tree" "$sha" "$size" "$tree/pkg"
if [ "$(cat "$tree/pkg/a.txt")" = hello ] && [ -n "$(find "$QUARANTINE" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
    ok "re-extract repairs tampered tree and quarantines old one"
else
    notok "re-extract repair"
fi

partial=$RUN/out/partial
mkdir -p "$partial/pkg"
printf 'stale\n' > "$partial/pkg/stale.txt"
ensure_archive_extracted "$F/safe.tar" "$partial" "$sha" "$size" "$partial/pkg"
if [ ! -e "$partial/pkg/stale.txt" ] && [ "$(cat "$partial/pkg/a.txt")" = hello ]; then
    ok "partial extraction is replaced atomically"
else
    notok "partial extraction replacement"
fi

# --- file tamper -----------------------------------------------------------
printf 'original\n' > "$RUN/out/file"
sum=$(sha256_of "$RUN/out/file")
expect_ok "check_file accepts matching hash" check_file "$RUN/out/file" "$sum"
printf 'changed\n' > "$RUN/out/file"
expect_fail "check_file rejects changed bytes" check_file "$RUN/out/file" "$sum"

# --- tree digest -----------------------------------------------------------
cp -a "$RUN/out/safe" "$RUN/out/safe-copy"
d1=$(tree_digest "$RUN/out/safe")
d2=$(tree_digest "$RUN/out/safe-copy")
if [ "$d1" = "$d2" ]; then ok "tree digest is stable across a copy"; else notok "tree digest copy"; fi
printf 'x' >> "$RUN/out/safe-copy/pkg/a.txt"
if [ "$d1" != "$(tree_digest "$RUN/out/safe-copy")" ]; then ok "tree digest changes on tamper"; else notok "tree digest tamper"; fi

# --- git checkout verification --------------------------------------------
repo=$RUN/fixtures/repo
git init -q "$repo"
git -C "$repo" config user.email t@example.test
git -C "$repo" config user.name test
printf 'a\n' > "$repo/a"
git -C "$repo" add a
git -C "$repo" commit -qm one
rev=$(git -C "$repo" rev-parse HEAD)
git clone -q "file://$repo" "$RUN/out/clone"
expect_ok "git checkout at pinned commit verifies" verify_git_checkout "$RUN/out/clone" "$rev"
printf 'b\n' >> "$RUN/out/clone/a"
expect_fail "git checkout rejects modified tracked file" verify_git_checkout "$RUN/out/clone" "$rev"
git -C "$RUN/out/clone" checkout -q -- a
ln -s ../../etc/passwd "$RUN/out/clone/escape"
expect_fail "git checkout rejects escaping symlink" verify_git_checkout "$RUN/out/clone" "$rev"
rm -f "$RUN/out/clone/escape"
printf 'extra\n' > "$RUN/out/clone/extra"
expect_fail "git checkout rejects unexpected untracked file" verify_git_checkout "$RUN/out/clone" "$rev"
expect_ok "git checkout allows an explicitly allowed untracked path" \
    verify_git_checkout "$RUN/out/clone" "$rev" --allow-untracked extra
rm -f "$RUN/out/clone/extra"
rm -f "$RUN/out/clone/a"
expect_fail "git checkout rejects deleted tracked file" verify_git_checkout "$RUN/out/clone" "$rev"
git -C "$RUN/out/clone" checkout -q -- a

# --- env quoting -----------------------------------------------------------
GN_BIN="$RUN/out/dir with space/gn \$dollar"
CLANG_DIR="$RUN/out/clang 'quote'"
WORK="$RUN/out/work dir"
env_content > "$RUN/out/env.expected"
if (
    # shellcheck disable=SC1091
    . "$RUN/out/env.expected"
    [ "$GN" = "$GN_BIN" ] && [ "$CLANG_BASE_PATH" = "$CLANG_DIR" ] &&
        [ "$CARGO_TARGET_DIR" = "$WORK/target" ]
); then
    ok "environment file shell-escapes spaces, dollars and quotes"
else
    notok "environment file quoting"
fi
ENVFILE="$WORK/build-env.sh"
write_env
if cmp -s <(env_content) "$ENVFILE"; then ok "write_env matches env_content"; else notok "write_env content"; fi

# --- isolated git/curl -----------------------------------------------------
cat > "$RUN/mockbin/git" <<'EOF'
#!/bin/sh
printf 'ARGS'
for a in "$@"; do printf ' [%s]' "$a"; done
printf '\n'
printf 'ENV CONFIG_GLOBAL=%s NOSYSTEM=%s PROMPT=%s ASKPASS=%s ALLOW=%s HOME=%s\n' \
    "${GIT_CONFIG_GLOBAL:-}" "${GIT_CONFIG_NOSYSTEM:-}" "${GIT_TERMINAL_PROMPT:-}" \
    "${GIT_ASKPASS:-}" "${GIT_ALLOW_PROTOCOL:-}" "${HOME:-}"
EOF
cat > "$RUN/mockbin/curl" <<'EOF'
#!/bin/sh
printf 'ARGS'
for a in "$@"; do printf ' [%s]' "$a"; done
printf '\n'
printf 'ENV HOME=%s CURL_HOME=%s HTTP_PROXY=%s HTTPS_PROXY=%s ALL_PROXY=%s\n' \
    "${HOME:-}" "${CURL_HOME:-}" "${http_proxy:-}" "${https_proxy:-}" "${ALL_PROXY:-}"
EOF
chmod +x "$RUN/mockbin/git" "$RUN/mockbin/curl"

out=$(PATH="$RUN/mockbin:$PATH" http_proxy=http://proxy.invalid:3128 HTTPS_PROXY=http://proxy.invalid:3128 \
    git_iso -C "$RUN/out/clone" rev-parse HEAD)
if contains "$out" "credential.helper=" && contains "$out" "http.followRedirects=false" &&
    contains "$out" "CONFIG_GLOBAL=/dev/null" && contains "$out" "NOSYSTEM=1" &&
    contains "$out" "PROMPT=0" && contains "$out" "ASKPASS=/bin/false" &&
    contains "$out" "-C" && contains "$out" "$RUN/out/clone" && contains "$out" "rev-parse"; then
    ok "git invocation is config- and proxy-isolated"
else
    notok "git isolation"; printf '%s\n' "$out" >&2
fi
case $out in
    *proxy.invalid*) notok "git inherited proxy environment" ;;
    *) ok "git did not inherit proxy environment" ;;
esac

ALLOWED_HOSTS="example.test"
out=$(PATH="$RUN/mockbin:$PATH" http_proxy=http://proxy.invalid:3128 \
    curl_iso https://example.test/file "$RUN/out/dl")
if contains "$out" "-q" && contains "$out" "--noproxy" && contains "$out" "--max-redirs" &&
    contains "$out" "0" && contains "$out" "--proto" && contains "$out" "=https" &&
    contains "$out" "HOME=$ISO_HOME"; then
    ok "curl invocation is config-, proxy- and redirect-isolated"
else
    notok "curl isolation"; printf '%s\n' "$out" >&2
fi
case $out in
    *proxy.invalid*) notok "curl inherited proxy environment" ;;
    *) ok "curl did not inherit proxy environment" ;;
esac
expect_fail "curl refuses non-HTTPS URL" curl_iso "http://example.test/x" "$RUN/out/dl2"
expect_fail "curl refuses unpinned host" curl_iso "https://evil.test/x" "$RUN/out/dl3"
# shellcheck disable=SC2034
ALLOWED_HOSTS="storage.googleapis.com static.crates.io"
expect_fail "git refuses unpinned host" fetch_commit bad https://evil.test/repo deadbeef

# --- verify-only must not create a missing cache --------------------------
missing=$RUN/missing-cache
if NEOTALES_V8_CACHE=$missing "$SCRIPT_DIR/prepare.sh" --verify-only >/dev/null 2>&1; then
    notok "verify-only rejected a missing cache"
else
    if [ ! -e "$missing" ]; then
        ok "verify-only rejects a missing cache without creating it"
    else
        notok "verify-only wrote to a missing cache"
    fi
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
