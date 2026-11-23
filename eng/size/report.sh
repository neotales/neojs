#!/usr/bin/env bash
# Print the ledger. The default view is the latest measurement per strict
# series; --trend shows longitudinal deltas per artifact family and labels
# which controls differ; --all prints every record in append order.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"

ROOT=$(size_repo_root)
LEDGER="$ROOT/eng/size/measurements.jsonl"
MODE=latest

size_require jq sha256sum mktemp column

usage() {
    cat <<'USAGE'
Usage: eng/size/report.sh [options]

Options:
  --ledger PATH   Read PATH instead of eng/size/measurements.jsonl.
  --trend         Print chronological deltas per artifact family, labelled
                  strict, controls-differ, conflict or missing.
  --all           Print every record in append order.
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
        --trend)
            MODE=trend
            shift
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

size_validate_ledger "$LEDGER" 0 allow-conflicts
size_report_ledger "$LEDGER" "$MODE"
