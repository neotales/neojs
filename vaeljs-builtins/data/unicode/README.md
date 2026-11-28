# Pinned Unicode 16.0 artifacts

## Case-folding (`vael:strings`)

`CaseFolding-16.0.0.txt` is the exact official Unicode Character Database
file used to generate `src/strings/case_fold.rs`.

* Source: <https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt>
* Version header: `# CaseFolding-16.0.0.txt`
* SHA-256: `6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb`
* Retrieved: 2026-11-23 from the official Unicode Public directory.
* License: Unicode License V3. The full notice is in
  [`../UNICODE-LICENSE.txt`](../UNICODE-LICENSE.txt); its SHA-256 is
  `e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96`.

The generator verifies the hash and version header before reading the file. It
selects only `C` and `S` entries; `F` and `T` entries are not part of simple
case folding.

```sh
# Re-download only from the exact official URL, then verify the pin.
curl -fsSL https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt \
  -o vaeljs-builtins/data/unicode/CaseFolding-16.0.0.txt
sha256sum -c <<'EOF'
6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb  vaeljs-builtins/data/unicode/CaseFolding-16.0.0.txt
EOF

# Regenerate or check the committed table without network access.
python3 vaeljs-builtins/scripts/generate_case_fold.py
python3 vaeljs-builtins/scripts/generate_case_fold.py --check
```

The exhaustive test in `src/strings/case_fold_test.rs` independently hashes the
artifact and compares every code point from `0` through `0x10FFFF` against the
generated table. It does not use the generator.

## One-code-point case mapping (`vael:chars`)

`UnicodeData-16.0.0.txt` and `SpecialCasing-16.0.0.txt` are the exact official
Unicode Character Database files used to generate `src/chars/case_map.rs`.

* `UnicodeData-16.0.0.txt`:
  * Source: <https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt>
  * First record: `0000;<control>;Cc;0;BN;;;;;N;NULL;;;;`
  * SHA-256: `ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f`
* `SpecialCasing-16.0.0.txt`:
  * Source: <https://www.unicode.org/Public/16.0.0/ucd/SpecialCasing.txt>
  * Version header: `# SpecialCasing-16.0.0.txt`
  * SHA-256: `8d5de354eef79f2395a54c9c7dcebbaf3d30fc962d0f85611ea97aa973a0c451`
* License: Unicode License V3; see [`../UNICODE-LICENSE.txt`](../UNICODE-LICENSE.txt).

The generator verifies both SHA-256 hashes and headers before reading the files,
extracts field 12 (`Simple_Uppercase_Mapping`) and field 13
(`Simple_Lowercase_Mapping`) from `UnicodeData-16.0.0.txt`, and omits any code
point whose unconditional `SpecialCasing-16.0.0.txt` mapping expands to more
than one code point (for example `U+0130` lowercase -> `0069 0307`) so that
expanding mappings return the input unchanged.

```sh
# Re-download only from the exact official URLs, then verify the pins.
curl -fsSL https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt \
  -o vaeljs-builtins/data/unicode/UnicodeData-16.0.0.txt
curl -fsSL https://www.unicode.org/Public/16.0.0/ucd/SpecialCasing.txt \
  -o vaeljs-builtins/data/unicode/SpecialCasing-16.0.0.txt
sha256sum -c <<'EOF'
ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f  vaeljs-builtins/data/unicode/UnicodeData-16.0.0.txt
8d5de354eef79f2395a54c9c7dcebbaf3d30fc962d0f85611ea97aa973a0c451  vaeljs-builtins/data/unicode/SpecialCasing-16.0.0.txt
EOF

# Regenerate or check the committed tables without network access.
python3 vaeljs-builtins/scripts/generate_case_map.py
python3 vaeljs-builtins/scripts/generate_case_map.py --check
```

The exhaustive test in `src/chars/case_map_test.rs` independently hashes both
artifacts and verifies every Unicode scalar value `0..=0x10FFFF` against
`UnicodeData-16.0.0.txt` and `SpecialCasing-16.0.0.txt`.
