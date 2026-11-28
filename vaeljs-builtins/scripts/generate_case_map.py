#!/usr/bin/env python3
"""Generate `src/chars/case_map.rs` from the pinned Unicode 16.0 artifacts.

The inputs are the exact official Unicode Character Database files:
- https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt
- https://www.unicode.org/Public/16.0.0/ucd/SpecialCasing.txt

The script refuses to run unless both artifacts' SHA-256 hashes and headers
match the pinned values. It extracts field 12 (`Simple_Uppercase_Mapping`) and
field 13 (`Simple_Lowercase_Mapping`) from `UnicodeData.txt`, and omits any
code point whose unconditional `SpecialCasing.txt` mapping expands to more than
one code point so that expanding mappings return the input unchanged.

Usage:
    python3 scripts/generate_case_map.py
    python3 scripts/generate_case_map.py --check

`--check` exits nonzero when the committed file is not exactly what the pinned
artifacts generate. It does not need a display or any network access.
"""

from __future__ import annotations

import difflib
import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
UNICODEDATA_ARTIFACT = ROOT / "data" / "unicode" / "UnicodeData-16.0.0.txt"
SPECIALCASING_ARTIFACT = ROOT / "data" / "unicode" / "SpecialCasing-16.0.0.txt"
TARGET = ROOT / "src" / "chars" / "case_map.rs"

UNICODEDATA_URL = "https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt"
UNICODEDATA_SHA256 = (
    "ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f"
)
UNICODEDATA_FIRST_LINE = "0000;<control>;Cc;0;BN;;;;;N;NULL;;;;"

SPECIALCASING_URL = "https://www.unicode.org/Public/16.0.0/ucd/SpecialCasing.txt"
SPECIALCASING_SHA256 = (
    "8d5de354eef79f2395a54c9c7dcebbaf3d30fc962d0f85611ea97aa973a0c451"
)
SPECIALCASING_HEADER = "# SpecialCasing-16.0.0.txt"


def verify_artifacts() -> None:
    ud_bytes = UNICODEDATA_ARTIFACT.read_bytes()
    ud_digest = hashlib.sha256(ud_bytes).hexdigest()
    if ud_digest != UNICODEDATA_SHA256:
        raise SystemExit(
            f"{UNICODEDATA_ARTIFACT} SHA-256 is {ud_digest}, expected {UNICODEDATA_SHA256}"
        )
    ud_first = ud_bytes.decode("utf-8").splitlines()[0]
    if ud_first != UNICODEDATA_FIRST_LINE:
        raise SystemExit(
            f"{UNICODEDATA_ARTIFACT} starts with {ud_first!r}, expected {UNICODEDATA_FIRST_LINE!r}"
        )

    sc_bytes = SPECIALCASING_ARTIFACT.read_bytes()
    sc_digest = hashlib.sha256(sc_bytes).hexdigest()
    if sc_digest != SPECIALCASING_SHA256:
        raise SystemExit(
            f"{SPECIALCASING_ARTIFACT} SHA-256 is {sc_digest}, expected {SPECIALCASING_SHA256}"
        )
    sc_first = sc_bytes.decode("utf-8").splitlines()[0]
    if sc_first != SPECIALCASING_HEADER:
        raise SystemExit(
            f"{SPECIALCASING_ARTIFACT} starts with {sc_first!r}, expected {SPECIALCASING_HEADER!r}"
        )


def case_mappings() -> tuple[dict[int, int], dict[int, int]]:
    upper: dict[int, int] = {}
    lower: dict[int, int] = {}
    for line_number, line in enumerate(
        UNICODEDATA_ARTIFACT.read_text(encoding="utf-8").splitlines(), 1
    ):
        if not line.strip():
            continue
        fields = line.split(";")
        if len(fields) != 15:
            raise SystemExit(
                f"UnicodeData line {line_number}: expected 15 fields, got {len(fields)}"
            )
        code = int(fields[0], 16)
        if fields[12]:
            target = int(fields[12], 16)
            if code in upper:
                raise SystemExit(
                    f"UnicodeData line {line_number}: duplicate uppercase mapping"
                )
            upper[code] = target
        if fields[13]:
            target = int(fields[13], 16)
            if code in lower:
                raise SystemExit(
                    f"UnicodeData line {line_number}: duplicate lowercase mapping"
                )
            lower[code] = target

    for line_number, line in enumerate(
        SPECIALCASING_ARTIFACT.read_text(encoding="utf-8").splitlines(), 1
    ):
        payload = line.split("#", 1)[0].strip()
        if not payload:
            continue
        fields = [field.strip() for field in payload.split(";")]
        if len(fields) < 4:
            raise SystemExit(
                f"SpecialCasing line {line_number}: expected at least 4 fields"
            )
        condition = fields[4] if len(fields) > 4 else ""
        if condition:
            continue
        code = int(fields[0], 16)
        lower_targets = fields[1].split()
        upper_targets = fields[3].split()
        if len(lower_targets) != 1:
            lower.pop(code, None)
        if len(upper_targets) != 1:
            upper.pop(code, None)

    return upper, lower


def ranges(mappings: dict[int, int]) -> list[tuple[int, int, int]]:
    merged: list[list[int]] = []
    for code, target in sorted(mappings.items()):
        if (
            merged
            and merged[-1][1] + 1 == code
            and merged[-1][2] + (merged[-1][1] - merged[-1][0]) + 1 == target
        ):
            merged[-1][1] = code
        else:
            merged.append([code, code, target])
    return [(start, end, target) for start, end, target in merged]


def format_ranges(name: str, doc: str, merged: list[tuple[int, int, int]]) -> list[str]:
    lines = [
        f"/// {doc}",
        f"pub(crate) const {name}: &[(u32, u32, u32)] = &[",
    ]
    for start, end, target in merged:
        lines.append(f"    (0x{start:04X}, 0x{end:04X}, 0x{target:04X}),")
    lines.append("];")
    return lines


def render(
    upper_ranges: list[tuple[int, int, int]],
    lower_ranges: list[tuple[int, int, int]],
) -> str:
    lines = [
        "//! Unicode 16.0 one-code-point case-mapping tables for `vael:chars`.",
        "//!",
        "//! Generated by `scripts/generate_case_map.py` from the pinned Unicode",
        "//! 16.0 `UnicodeData.txt` and `SpecialCasing.txt` artifacts. Do not edit by hand.",
        "//!",
        f"//! Sources: {UNICODEDATA_URL}, {SPECIALCASING_URL}",
        f"//! UnicodeData SHA-256: {UNICODEDATA_SHA256}",
        f"//! SpecialCasing SHA-256: {SPECIALCASING_SHA256}",
        "//! License: Unicode License V3; see `data/UNICODE-LICENSE.txt`.",
        "//! Selection: `UnicodeData.txt` field 12 (`Simple_Uppercase_Mapping`) and",
        "//! field 13 (`Simple_Lowercase_Mapping`), excluding code points whose",
        "//! unconditional `SpecialCasing.txt` mapping expands to more than one code",
        "//! point (such as `U+0130` lowercase -> `0069 0307`). Ranges merge only",
        "//! when both the source and target code points are consecutive.",
        "//!",
        "//! Both classification (`unicode-general-category`) and case mapping",
        "//! (`toLower` / `toUpper`) in `vael:chars` are pinned to Unicode 16.0,",
        "//! independent of the Rust compiler's standard library Unicode version.",
        "",
    ]
    lines.extend(
        format_ranges(
            "UPPER_RANGES",
            "The one-code-point uppercase mapping ranges, sorted by inclusive start.",
            upper_ranges,
        )
    )
    lines.append("")
    lines.extend(
        format_ranges(
            "LOWER_RANGES",
            "The one-code-point lowercase mapping ranges, sorted by inclusive start.",
            lower_ranges,
        )
    )
    lines.extend(
        [
            "",
            "fn lookup_range(ranges: &[(u32, u32, u32)], point: u32) -> u32 {",
            "    let index = ranges.partition_point(|&(start, _, _)| start <= point);",
            "    if index == 0 {",
            "        return point;",
            "    }",
            "    let (start, end, target) = ranges[index - 1];",
            "    if point <= end {",
            "        target + (point - start)",
            "    } else {",
            "        point",
            "    }",
            "}",
            "",
            "/// Returns the Unicode 16.0 one-code-point uppercase mapping of `point`.",
            "///",
            "/// A scalar with no mapping or a multi-code-point expansion returns itself.",
            f"/// The lookup is a binary search over {len(upper_ranges)} merged ranges.",
            "#[must_use]",
            "pub(crate) fn simple_upper(point: u32) -> u32 {",
            "    if point < 0x80 {",
            "        return if (b'a' as u32..=b'z' as u32).contains(&point) {",
            "            point - 0x20",
            "        } else {",
            "            point",
            "        };",
            "    }",
            "    lookup_range(UPPER_RANGES, point)",
            "}",
            "",
            "/// Returns the Unicode 16.0 one-code-point lowercase mapping of `point`.",
            "///",
            "/// A scalar with no mapping or a multi-code-point expansion returns itself.",
            f"/// The lookup is a binary search over {len(lower_ranges)} merged ranges.",
            "#[must_use]",
            "pub(crate) fn simple_lower(point: u32) -> u32 {",
            "    if point < 0x80 {",
            "        return if (b'A' as u32..=b'Z' as u32).contains(&point) {",
            "            point + 0x20",
            "        } else {",
            "            point",
            "        };",
            "    }",
            "    lookup_range(LOWER_RANGES, point)",
            "}",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    verify_artifacts()
    upper, lower = case_mappings()
    rendered = render(ranges(upper), ranges(lower))
    if "--check" in sys.argv[1:]:
        current = TARGET.read_text(encoding="utf-8")
        if current == rendered:
            print(f"{TARGET} is up to date")
            return 0
        diff = difflib.unified_diff(
            current.splitlines(),
            rendered.splitlines(),
            fromfile=str(TARGET),
            tofile="generated",
            lineterm="",
        )
        print("\n".join(diff))
        return 1
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(rendered, encoding="utf-8")
    print(f"wrote {TARGET}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
