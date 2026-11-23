#!/usr/bin/env bash
# Print the current size series from the ledger. The default view is the
# latest measurement for each package/target/toolchain/feature set; --all
# prints every record in append order.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

ROOT=$(size_repo_root)
LEDGER="$ROOT/eng/size/measurements.jsonl"
MODE=latest

size_require jq sha256sum column

usage() {
    cat <<'USAGE'
Usage: eng/size/report.sh [options]

Options:
  --ledger PATH   Read PATH instead of eng/size/measurements.jsonl.
  --all           Print every record instead of the latest per series.
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
        --all)
            MODE=all
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

size_validate_ledger "$LEDGER" 0
size_report_ledger "$LEDGER" "$MODE"
