# Third-party notices

## Unicode Character Database — CaseFolding.txt 16.0.0

* File: `data/unicode/CaseFolding-16.0.0.txt`
* Source: <https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt>
* Version header: `# CaseFolding-16.0.0.txt`
* SHA-256: `6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb`
* Copyright © 2024 Unicode, Inc.
* License: Unicode License V3. The full notice is in
  [`data/UNICODE-LICENSE.txt`](data/UNICODE-LICENSE.txt); its SHA-256 is
  `e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96`.

Only the `C` and `S` simple case-folding entries are selected. The artifact is
not modified; `scripts/generate_case_fold.py` reads it in place and emits the
Rust range table. The table is derived data and is distributed under the same
Unicode License V3 notice.
