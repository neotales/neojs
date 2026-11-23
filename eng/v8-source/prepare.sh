#!/usr/bin/env bash
# Acquire, verify and assemble the exact inputs for a from-source v8 build.
#
# The crates.io archive `v8` 150.4.0 omits GN, the Chromium Rust toolchain, the
# `third_party/rust` vendor tree and the ICU data blobs. Each has an exact
# revision pinned by the crate's own DEPS; this script fetches that revision and
# checks a hash, size or git tree before use.
#
# Security properties:
#   * downloads use an isolated curl config: no ~/.curlrc, no netrc, no proxy
#     environment, HTTPS only, no redirects, host allowlist;
#   * git uses an isolated config: no system/global config, a controlled local
#     config with no includes/remotes/url rewrite/hooks/fsmonitor, no credential
#     helper or askpass, HTTPS only, redirects disabled; every fetch names the
#     pinned URL explicitly;
#   * archives are validated for traversal, escaping links, device members and
#     duplicate/ancestor-symlink tricks before extraction;
#   * extraction is staged under the cache and swapped in atomically; a tree
#     that fails its recorded digest is re-extracted, never patched in place;
#   * --verify-only performs no writes, opens no network, and executes no
#     downloaded binary; it checks hashes, git tree content and recorded tree
#     digests only. It does run host git/python3/coreutils.
#   * recorded tree digests are drift detection, not signatures: an attacker
#     who can write the cache can forge them. The cryptographic roots are the
#     crate checksum and the git commit hashes, which verify-only re-derives.
#
# It never reads a credential and never writes to the cargo registry. It does
# read the cargo registry cache, read-only, to reuse the crate archive cargo
# already verified.
#
# `V8_PREPARE_TEST_MODE=1 V8_PREPARE_TEST_PINS=<file>` is a test-only seam used
# by test.sh; production use must leave both unset.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
VALIDATOR=$SCRIPT_DIR/validate_tar.py
VERIFY_TREE=$SCRIPT_DIR/verify_git_tree.py
MATERIALIZE=$SCRIPT_DIR/materialize_git_tree.py

LIB_MODE=${V8_PREPARE_LIB:-0}
VERIFY_ONLY=0
TEST_MODE=0

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

stamp_get() {
    local file=$1 key=$2
    [ -f "$file" ] || return 1
    sed -n "s/^${key} //p" "$file" | head -1
}

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

ALLOWED_HOSTS="storage.googleapis.com static.crates.io"
ALLOWED_GIT_HOSTS="gn.googlesource.com chromium.googlesource.com"
GIT_PROTOCOLS=https

if [ -n "${V8_PREPARE_TEST_PINS:-}" ]; then
    [ "${V8_PREPARE_TEST_MODE:-0}" = 1 ] ||
        die "V8_PREPARE_TEST_PINS is test-only; set V8_PREPARE_TEST_MODE=1"
    TEST_MODE=1
    GIT_PROTOCOLS=file:https
    # shellcheck source=/dev/null
    . "$V8_PREPARE_TEST_PINS"
fi

CACHE=${NEOTALES_V8_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/neotales/neojs-v8-source}

init_paths() {
    SRC=$CACHE/src
    DOWNLOADS=$CACHE/downloads
    WORK=$CACHE/work
    STAMPS=$CACHE/.stamps
    STAGING=$CACHE/.staging
    QUARANTINE=$CACHE/.quarantine
    FAILED=$CACHE/.failed
    TMP=$CACHE/.tmp
    ISO_HOME=$CACHE/.isolated-home
    GN_DIR=$SRC/gn
    RUST_DIR=$SRC/rust
    ICU_DIR=$SRC/icu
    GN_BIN=$GN_DIR/out/gn
    PRISTINE_ROOT=$SRC/v8-pristine
    PRISTINE_DIR=$PRISTINE_ROOT/v8-$CRATE_VERSION
    TOOLCHAIN_DIR=$SRC/rust-toolchain
    CLANG_DIR=$SRC/clang
    PREPARED=$SRC/v8/v8-$CRATE_VERSION
    ENVFILE=$WORK/build-env.sh
}

# --- Verification primitives ----------------------------------------------
matches_file() {
    local path=$1 want_sha=$2 want_size=${3:-}
    [ -f "$path" ] || return 1
    [ -z "$want_size" ] || [ "$(size_of "$path")" = "$want_size" ] || return 1
    [ "$(sha256_of "$path")" = "$want_sha" ]
}

check_file() {
    local path=$1 want_sha=$2 want_size=${3:-}
    [ -f "$path" ] || die "missing file: $path"
    if [ -n "$want_size" ]; then
        [ "$(size_of "$path")" = "$want_size" ] ||
            die "size mismatch for $path: got $(size_of "$path"), want $want_size"
    fi
    [ "$(sha256_of "$path")" = "$want_sha" ] || die "sha256 mismatch for $path"
}

tree_digest() {
    local dir=$1
    [ -d "$dir" ] || die "missing tree: $dir"
    (
        cd "$dir"
        {
            find . -type d -printf 'D %m %p\n'
            find . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum
            find . -type f -printf 'F %m %p\n'
            find . -type l -printf 'L %m %p -> %l\n'
        } | LC_ALL=C sort | sha256sum | cut -d' ' -f1
    )
}

# --- Isolated network and VCS ---------------------------------------------
curl_iso() {
    local url=$1 out=$2
    local rest=${url#https://}
    [ "$rest" != "$url" ] || die "refusing non-HTTPS URL: $url"
    local host=${rest%%/*}
    case " $ALLOWED_HOSTS " in
        *" $host "*) ;;
        *) die "refusing unpinned host: $host" ;;
    esac
    env -i PATH="$PATH" HOME="$ISO_HOME" CURL_HOME="$ISO_HOME" LC_ALL=C \
        curl -q --fail --silent --show-error \
        --proto '=https' --proto-redir '=https' --max-redirs 0 --tlsv1.2 \
        --noproxy '*' --connect-timeout 30 --retry 2 \
        --output "$out.part" "$url"
}

git_iso() {
    env -i PATH="$PATH" HOME="$ISO_HOME" XDG_CONFIG_HOME="$ISO_HOME/.config" LC_ALL=C \
        GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
        GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=/bin/false SSH_ASKPASS=/bin/false \
        GIT_SSH_COMMAND=false GIT_ALLOW_PROTOCOL="$GIT_PROTOCOLS" \
        GIT_PROTOCOL_FROM_USER=0 GIT_OPTIONAL_LOCKS=0 \
        git --no-optional-locks \
        -c credential.helper= -c core.askPass= -c core.hooksPath=/nonexistent \
        -c core.fsmonitor=false -c core.autocrlf=false -c core.safecrlf=false \
        -c http.followRedirects=false -c http.proxy= -c https.proxy= \
        -c http.sslVerify=true -c protocol.version=2 "$@"
}

check_git_url() {
    local url=$1
    case $url in
        https://*)
            local host=${url#https://}
            host=${host%%/*}
            case " $ALLOWED_GIT_HOSTS " in
                *" $host "*) ;;
                *) die "refusing unpinned git host: $host" ;;
            esac
            ;;
        file://*) [ "$TEST_MODE" = 1 ] || die "refusing non-HTTPS git URL: $url" ;;
        *) die "refusing git URL: $url" ;;
    esac
}

git_config_content() {
    printf '[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\tlogallrefupdates = true\n'
}

write_git_config() {
    local dir=$1
    [ "$VERIFY_ONLY" = 0 ] || die "internal error: writing .git/config during verify-only"
    git_config_content > "$dir/.git/config"
    chmod 644 "$dir/.git/config"
}

verify_git_config() {
    local dir=$1
    [ -d "$dir/.git" ] || die "missing .git directory: $dir"
    [ ! -L "$dir/.git" ] || die ".git is a symlink: $dir"
    [ -f "$dir/.git/config" ] || die "missing .git/config: $dir"
    [ ! -L "$dir/.git/config" ] || die ".git/config is a symlink: $dir"
    cmp -s <(git_config_content) "$dir/.git/config" ||
        die "unexpected .git/config (includes/url rewrites/remotes): $dir"
    [ ! -e "$dir/.git/config.worktree" ] || die "unexpected .git/config.worktree: $dir"
    if find "$dir/.git" -type l -print -quit | grep -q .; then
        die "symlink inside .git: $dir"
    fi
    if find "$dir/.git/hooks" -type f -perm /111 ! -name '*.sample' -print -quit |
        grep -q .; then
        die "executable hook inside .git: $dir"
    fi
}

verify_git_checkout() {
    local dir=$1 rev=$2
    shift 2
    verify_git_config "$dir"
    local head
    head=$(git_iso -C "$dir" rev-parse HEAD) || die "cannot read HEAD in $dir"
    [ "$head" = "$rev" ] || die "commit mismatch in $dir: got $head, want $rev"
    git_iso -C "$dir" fsck --no-dangling --no-progress >/dev/null 2>&1 ||
        die "git object check failed in $dir"
    env -i PATH="$PATH" HOME="$ISO_HOME" LC_ALL=C \
        python3 "$VERIFY_TREE" "$dir" "$rev" --home "$ISO_HOME" "$@" ||
        die "working tree does not match $rev in $dir"
}

# --- Safe extraction -------------------------------------------------------
quarantine() {
    local path=$1
    [ -e "$path" ] || return 0
    mkdir -p "$QUARANTINE"
    local base dest
    base=$(basename "$path")
    dest=$QUARANTINE/$base.$(date -u +%Y%m%dT%H%M%SZ)
    [ ! -e "$dest" ] || die "quarantine collision: $dest"
    mv "$path" "$dest"
}

safe_extract() {
    local archive=$1 dest=$2
    [ "$VERIFY_ONLY" = 0 ] || die "internal error: extraction during verify-only"
    [ -f "$archive" ] || die "missing archive: $archive"
    mkdir -p "$STAGING"
    local stage
    stage=$(mktemp -d "$STAGING/extract.XXXXXX")
    if ! python3 "$VALIDATOR" --extract "$archive" "$stage"; then
        mkdir -p "$FAILED"
        mv "$stage" "$FAILED/extract.$(date -u +%Y%m%dT%H%M%SZ).$$"
        die "archive failed validation: $archive"
    fi
    quarantine "$dest"
    mkdir -p "$(dirname "$dest")"
    mv "$stage" "$dest"
}

ensure_archive_extracted() {
    local archive=$1 dest=$2 sha=$3 size=$4 tree=${5:-$2}
    local base stamp
    base=$(basename "$dest")
    stamp=$STAMPS/$base.stamp
    if [ "$(stamp_get "$stamp" archive_sha256 || true)" = "$sha" ] && [ -d "$tree" ]; then
        local recorded actual
        recorded=$(stamp_get "$stamp" tree_sha256 || true)
        actual=$(tree_digest "$tree")
        if [ -n "$recorded" ] && [ "$recorded" = "$actual" ]; then
            return 0
        fi
        [ "$VERIFY_ONLY" = 0 ] || die "tree digest mismatch: $tree"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified tree: $tree"
    check_file "$archive" "$sha" "$size"
    safe_extract "$archive" "$dest"
    mkdir -p "$STAMPS"
    {
        printf 'archive_sha256 %s\n' "$sha"
        printf 'tree_sha256 %s\n' "$(tree_digest "$tree")"
    } > "$stamp"
}

# --- Acquisition -----------------------------------------------------------
download() {
    local url=$1 out=$2 sha=$3 size=${4:-}
    if matches_file "$out" "$sha" "$size"; then
        return 0
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified download: $out"
    mkdir -p "$(dirname "$out")"
    printf 'fetch %s\n' "$url"
    curl_iso "$url" "$out"
    check_file "$out.part" "$sha" "$size"
    mv "$out.part" "$out"
}

fetch_commit() {
    local name=$1 url=$2 rev=$3
    shift 3
    local dir=$SRC/$name
    if [ -d "$dir/.git" ]; then
        local err
        if err=$( (verify_git_checkout "$dir" "$rev" "$@") 2>&1 ); then
            return 0
        fi
        [ "$VERIFY_ONLY" = 0 ] ||
            die "unverified git checkout: $dir @ $rev: ${err:-unknown error}"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified git checkout: $dir @ $rev"
    check_git_url "$url"
    printf 'fetch %s @ %s\n' "$url" "$rev"
    quarantine "$dir"
    mkdir -p "$dir"
    git_iso -C "$dir" init -q
    write_git_config "$dir"
    git_iso -C "$dir" fetch -q --no-tags --depth 1 "$url" "$rev"
    local fetched
    fetched=$(git_iso -C "$dir" rev-parse FETCH_HEAD) || die "cannot read FETCH_HEAD in $dir"
    [ "$fetched" = "$rev" ] || die "fetched commit mismatch in $dir: got $fetched, want $rev"
    git_iso -C "$dir" update-ref refs/heads/main "$rev"
    git_iso -C "$dir" symbolic-ref HEAD refs/heads/main
    env -i PATH="$PATH" HOME="$ISO_HOME" LC_ALL=C \
        python3 "$MATERIALIZE" "$dir" "$rev" "$dir" --allow-existing-git --home "$ISO_HOME" ||
        die "cannot materialise $rev into $dir"
    verify_git_checkout "$dir" "$rev" "$@"
}

# --- Inputs ----------------------------------------------------------------
ensure_pristine() {
    # Prefer the cargo-verified crate archive; never write to the registry.
    local archive=$DOWNLOADS/v8-$CRATE_VERSION.crate
    local candidate
    shopt -s nullglob
    for candidate in "$HOME"/.cargo/registry/cache/*/"v8-$CRATE_VERSION.crate"; do
        if matches_file "$candidate" "$CRATE_SHA256"; then
            archive=$candidate
            break
        fi
    done
    shopt -u nullglob
    download "$CRATE_URL" "$archive" "$CRATE_SHA256"
    local stamp=$STAMPS/pristine.stamp
    if [ "$(stamp_get "$stamp" archive_sha256 || true)" = "$CRATE_SHA256" ] && [ -d "$PRISTINE_DIR" ]; then
        local recorded actual
        recorded=$(stamp_get "$stamp" tree_sha256 || true)
        actual=$(tree_digest "$PRISTINE_DIR")
        if [ -n "$recorded" ] && [ "$recorded" = "$actual" ]; then
            return 0
        fi
        [ "$VERIFY_ONLY" = 0 ] || die "tree digest mismatch: $PRISTINE_DIR"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified crate tree: $PRISTINE_DIR"
    safe_extract "$archive" "$PRISTINE_ROOT"
    [ -d "$PRISTINE_DIR" ] || die "crate archive did not contain v8-$CRATE_VERSION"
    mkdir -p "$STAMPS"
    {
        printf 'archive_sha256 %s\n' "$CRATE_SHA256"
        printf 'tree_sha256 %s\n' "$(tree_digest "$PRISTINE_DIR")"
    } > "$stamp"
}

ensure_toolchain() {
    local archive=$DOWNLOADS/$TOOLCHAIN_NAME
    download "$TOOLCHAIN_URL" "$archive" "$TOOLCHAIN_SHA256" "$TOOLCHAIN_SIZE"
    local stamp=$STAMPS/toolchain.stamp
    if [ "$(stamp_get "$stamp" archive_sha256 || true)" = "$TOOLCHAIN_SHA256" ] && [ -f "$TOOLCHAIN_DIR/bin/rustc" ]; then
        local recorded actual
        recorded=$(stamp_get "$stamp" tree_sha256 || true)
        actual=$(tree_digest "$TOOLCHAIN_DIR")
        if [ -n "$recorded" ] && [ "$recorded" = "$actual" ]; then
            [ "$(cat "$TOOLCHAIN_DIR/.rusty_v8_version")" = "$TOOLCHAIN_URL" ] ||
                die "stale Rust toolchain sentinel"
            if [ "$VERIFY_ONLY" = 0 ] && [ ! -x "$TOOLCHAIN_DIR/bin/rustc" ]; then
                die "Rust toolchain lost its executable bit; remove its stamp to re-extract"
            fi
            return 0
        fi
        [ "$VERIFY_ONLY" = 0 ] || die "tree digest mismatch: $TOOLCHAIN_DIR"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified Rust toolchain: $TOOLCHAIN_DIR"
    safe_extract "$archive" "$TOOLCHAIN_DIR"
    printf '%s' "$TOOLCHAIN_URL" > "$TOOLCHAIN_DIR/.rusty_v8_version"
    mkdir -p "$STAMPS"
    {
        printf 'archive_sha256 %s\n' "$TOOLCHAIN_SHA256"
        printf 'tree_sha256 %s\n' "$(tree_digest "$TOOLCHAIN_DIR")"
    } > "$stamp"
}

ensure_clang() {
    local archive=$DOWNLOADS/$CLANG_NAME
    download "$CLANG_URL" "$archive" "$CLANG_SHA256" "$CLANG_SIZE"
    ensure_archive_extracted "$archive" "$CLANG_DIR" "$CLANG_SHA256" "$CLANG_SIZE" "$CLANG_DIR"
    if [ "$VERIFY_ONLY" = 0 ] && [ ! -x "$CLANG_DIR/bin/clang" ]; then
        die "Chromium clang lost its executable bit; remove src/clang.stamp to re-extract"
    fi
}

ensure_gn() {
    local stamp=$STAMPS/gn.stamp
    local dir=$GN_DIR
    if [ -d "$dir/.git" ]; then
        verify_git_checkout "$dir" "$GN_REV" --allow-extra out/
    else
        [ "$VERIFY_ONLY" = 0 ] || die "missing GN checkout: $dir"
    fi
    if [ -f "$GN_BIN" ] &&
       [ "$(stamp_get "$stamp" commit || true)" = "$GN_REV" ] &&
       [ "$(stamp_get "$stamp" binary_sha256 || true)" = "$(sha256_of "$GN_BIN")" ]; then
        if [ "$VERIFY_ONLY" = 1 ] || [ -x "$GN_BIN" ]; then
            return 0
        fi
        die "GN binary lost its executable bit; remove its stamp to rebuild"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified GN binary: $GN_BIN"
    if [ "$(git_iso -C "$dir" rev-parse --is-shallow-repository)" = true ]; then
        check_git_url "$GN_URL"
        git_iso -C "$dir" fetch -q --unshallow --tags "$GN_URL"
    fi
    (
        cd "$dir"
        env -i PATH="$PATH" HOME="$ISO_HOME" LC_ALL=C TMPDIR="$TMP" \
            GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
            python3 build/gen.py --out-path out
        env -i PATH="$PATH" HOME="$ISO_HOME" LC_ALL=C TMPDIR="$TMP" \
            ninja -C out gn
    )
    mkdir -p "$STAMPS"
    {
        printf 'commit %s\n' "$GN_REV"
        printf 'binary_sha256 %s\n' "$(sha256_of "$GN_BIN")"
        printf 'version %s\n' "$("$GN_BIN" --version)"
    } > "$stamp"
}

ensure_rust_and_icu_checkouts() {
    fetch_commit rust "$RUST_URL" "$RUST_REV"
    fetch_commit icu "$ICU_URL" "$ICU_REV"
    check_file "$ICU_DIR/common/icudtl.dat" "$ICU_LITTLE_DAT_SHA256"
    check_file "$ICU_DIR/common/icudtb.dat" "$ICU_BIG_DAT_SHA256"
}

ensure_prepared() {
    local stamp=$STAMPS/prepared.stamp
    if [ -d "$PREPARED" ] &&
       [ "$(stamp_get "$stamp" crate_sha256 || true)" = "$CRATE_SHA256" ] &&
       [ "$(stamp_get "$stamp" rust_commit || true)" = "$RUST_REV" ] &&
       [ "$(stamp_get "$stamp" icu_commit || true)" = "$ICU_REV" ] &&
       [ "$(stamp_get "$stamp" toolchain_sha256 || true)" = "$TOOLCHAIN_SHA256" ] &&
       [ "$(stamp_get "$stamp" clang_sha256 || true)" = "$CLANG_SHA256" ]; then
        local recorded actual
        recorded=$(stamp_get "$stamp" tree_sha256 || true)
        actual=$(tree_digest "$PREPARED")
        if [ -n "$recorded" ] && [ "$recorded" = "$actual" ]; then
            return 0
        fi
        [ "$VERIFY_ONLY" = 0 ] || die "tree digest mismatch: $PREPARED"
    fi
    [ "$VERIFY_ONLY" = 0 ] || die "missing or unverified prepared tree: $PREPARED"

    [ -d "$PRISTINE_DIR" ] || die "pristine tree missing; run ensure_pristine first"
    [ -x "$TOOLCHAIN_DIR/bin/rustc" ] || die "Rust toolchain missing"
    mkdir -p "$STAGING"
    local stage
    stage=$(mktemp -d "$STAGING/prepared.XXXXXX")
    cp -a "$PRISTINE_DIR/." "$stage/"
    rm -rf "$stage/third_party/rust" "$stage/third_party/rust-toolchain"
    cp -a "$RUST_DIR" "$stage/third_party/rust"
    rm -rf "$stage/third_party/rust/.git"
    cp -a "$TOOLCHAIN_DIR" "$stage/third_party/rust-toolchain"
    mkdir -p "$stage/third_party/icu/common"
    cp -f "$ICU_DIR/common/icudtl.dat" "$stage/third_party/icu/common/icudtl.dat"
    cp -f "$ICU_DIR/common/icudtb.dat" "$stage/third_party/icu/common/icudtb.dat"
    quarantine "$PREPARED"
    mkdir -p "$(dirname "$PREPARED")"
    mv "$stage" "$PREPARED"
    mkdir -p "$STAMPS"
    {
        printf 'crate_sha256 %s\n' "$CRATE_SHA256"
        printf 'rust_commit %s\n' "$RUST_REV"
        printf 'icu_commit %s\n' "$ICU_REV"
        printf 'toolchain_sha256 %s\n' "$TOOLCHAIN_SHA256"
        printf 'clang_sha256 %s\n' "$CLANG_SHA256"
        printf 'tree_sha256 %s\n' "$(tree_digest "$PREPARED")"
    } > "$stamp"
}

# --- Environment file ------------------------------------------------------
env_content() {
    printf 'export V8_FROM_SOURCE=1\n'
    printf 'export GN=%q\n' "$GN_BIN"
    printf 'export NINJA=%q\n' "$(command -v ninja)"
    printf 'export CLANG_BASE_PATH=%q\n' "$CLANG_DIR"
    printf 'export CARGO_TARGET_DIR=%q\n' "$WORK/target"
    if [ -f /usr/lib/libclang.so ]; then
        printf 'export LIBCLANG_PATH=/usr/lib\n'
    fi
}

write_env() {
    [ "$VERIFY_ONLY" = 0 ] || die "internal error: writing during verify-only"
    mkdir -p "$WORK"
    env_content > "$ENVFILE"
}

verify_env() {
    [ -f "$ENVFILE" ] || die "missing environment file: $ENVFILE"
    cmp -s <(env_content) "$ENVFILE" || die "environment file is stale or unquoted: $ENVFILE"
}

# --- Entry points ----------------------------------------------------------
prepare() {
    mkdir -p "$SRC" "$DOWNLOADS" "$WORK" "$STAMPS" "$STAGING" "$QUARANTINE" "$FAILED" "$TMP"
    mkdir -p "$ISO_HOME"
    chmod 700 "$ISO_HOME"
    ensure_pristine
    ensure_rust_and_icu_checkouts
    ensure_toolchain
    ensure_clang
    fetch_commit gn "$GN_URL" "$GN_REV" --allow-extra out/
    ensure_gn
    ensure_prepared
    write_env
    printf 'prepared v8 %s in %s\n' "$CRATE_VERSION" "$CACHE"
    printf '  prepared  %s\n' "$PREPARED"
    printf '  env       %s\n' "$ENVFILE"
}

verify_all() {
    ensure_pristine
    ensure_rust_and_icu_checkouts
    ensure_toolchain
    ensure_clang
    fetch_commit gn "$GN_URL" "$GN_REV" --allow-extra out/
    ensure_gn
    ensure_prepared
    verify_env
    printf 'verified v8 %s inputs in %s (read-only)\n' "$CRATE_VERSION" "$CACHE"
    printf '  prepared  %s\n' "$PREPARED"
    printf '  gn        %s\n' "$(stamp_get "$STAMPS/gn.stamp" version)"
    printf '  toolchain sha256 %s\n' "$TOOLCHAIN_SHA256"
}

main() {
    while [ $# -gt 0 ]; do
        case $1 in
            --verify-only) VERIFY_ONLY=1 ;;
            -h|--help)
                printf 'usage: %s [--verify-only]\n' "$0"
                return 0
                ;;
            *) die "unknown argument: $1" ;;
        esac
        shift
    done
    if [ "$VERIFY_ONLY" = 1 ]; then
        need git
        need sha256sum
        need find
        need xargs
        need sort
        need cmp
        need readlink
        need realpath
        need stat
        need date
        need sed
        need python3
        need ninja
        init_paths
        verify_all
    else
        need git
        need curl
        need sha256sum
        need find
        need xargs
        need sort
        need cmp
        need readlink
        need realpath
        need stat
        need date
        need sed
        need python3
        need tar
        need ninja
        need mktemp
        command -v clang++ >/dev/null 2>&1 || command -v g++ >/dev/null 2>&1 ||
            die "missing host tool: clang++ or g++"
        init_paths
        prepare
    fi
}

if [ "$LIB_MODE" != 1 ]; then
    main "$@"
fi
