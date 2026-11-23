# Pinned Unicode 16.0 case-folding artifact

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
  -o neojs-builtins/data/unicode/CaseFolding-16.0.0.txt
sha256sum -c <<'EOF'
6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb  neojs-builtins/data/unicode/CaseFolding-16.0.0.txt
EOF

# Regenerate or check the committed table without network access.
python3 neojs-builtins/scripts/generate_case_fold.py
python3 neojs-builtins/scripts/generate_case_fold.py --check
```

The exhaustive test in `src/strings/case_fold.rs` independently hashes the
artifact and compares every code point from `0` through `0x10FFFF` against the
generated table. It does not use the generator.
