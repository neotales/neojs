#!/usr/bin/env python3
"""Add a verified `v8` path patch to a Cargo workspace manifest.

Usage:
    patch_workspace.py CARGO_TOML V8_PATH

The `[patch.crates-io]` section must already exist; a missing section is an
error, not a reason to append one silently. The path is serialised as a TOML
basic string with `json.dumps` (JSON escapes are TOML-compatible for quotes,
backslashes and control characters) and the result is re-parsed with `tomllib`
before the file is replaced.
"""

import json
import pathlib
import re
import sys
import tomllib

HEADER = re.compile(r"^\[patch\.crates-io\][ \t]*(?:#.*)?$", re.MULTILINE)


def main():
    if len(sys.argv) != 3:
        print("usage: patch_workspace.py CARGO_TOML V8_PATH", file=sys.stderr)
        return 2
    manifest = pathlib.Path(sys.argv[1])
    v8_path = sys.argv[2]

    original = manifest.read_text(encoding="utf-8")
    data = tomllib.loads(original)
    patch = data.get("patch", {})
    table = patch.get("crates-io")
    if not isinstance(table, dict):
        print("patch_workspace: [patch.crates-io] section is missing", file=sys.stderr)
        return 2
    if "v8" in table:
        print("patch_workspace: [patch.crates-io] already has a v8 entry", file=sys.stderr)
        return 2

    match = HEADER.search(original)
    if not match:
        print("patch_workspace: [patch.crates-io] header not found", file=sys.stderr)
        return 2

    escaped = json.dumps(v8_path)
    inserted = original[: match.end()] + f"\nv8 = {{ path = {escaped} }}" + original[match.end():]
    parsed = tomllib.loads(inserted)
    got = parsed.get("patch", {}).get("crates-io", {}).get("v8", {}).get("path")
    if got != v8_path:
        print("patch_workspace: serialised path did not round-trip", file=sys.stderr)
        return 1

    manifest.write_text(inserted, encoding="utf-8")
    print(f"patched {manifest}: v8 = {{ path = {escaped} }}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
