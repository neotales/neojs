#!/usr/bin/env bash
# The size-tracking gate: deterministic tracker tests plus validation of the
# committed ledger. Cheap enough to run on every change to eng/size/.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

bash "$SCRIPT_DIR/test.sh"
if [ -s "$SCRIPT_DIR/measurements.jsonl" ]; then
    bash "$SCRIPT_DIR/validate.sh"
else
    # An empty ledger is a tracker with no baseline yet, not a green history.
    # Say so, and still exercise the validator's empty-ledger path.
    printf 'size: ledger is empty (no measurements recorded yet)\n' >&2
    bash "$SCRIPT_DIR/validate.sh" --allow-empty
fi
printf 'size: check passed\n'
