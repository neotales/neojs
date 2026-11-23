#!/usr/bin/env bash
# Validate eng/size/measurements.jsonl: schema, field types, key integrity,
# duplicate keys, and non-reproducible build contexts.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

ROOT=$(size_repo_root)
LEDGER="$ROOT/eng/size/measurements.jsonl"
ALLOW_EMPTY=0

size_require jq sha256sum

usage() {
    cat <<'USAGE'
Usage: eng/size/validate.sh [options]

Options:
  --ledger PATH   Validate PATH instead of eng/size/measurements.jsonl.
  --allow-empty   Accept a zero-length ledger.
  -h, --help      Show this help.
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --ledger)
            [ "$#" -ge 2 ] || { usage >&2; exit 2; }
            LEDGER=$2
            shift 2
            ;;
        --allow-empty)
            ALLOW_EMPTY=1
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

size_validate_ledger "$LEDGER" "$ALLOW_EMPTY"
printf 'size: validated %s\n' "$LEDGER"
