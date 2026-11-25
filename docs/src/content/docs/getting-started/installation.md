---
title: Installation
description: Download prebuilt vjs and vjs-lite binaries across 6 targets, verify SHA-256 checksums, or build from source.
---

VaelJS ships two standalone CLI binaries on [GitHub Releases](https://github.com/neotales/vaeljs/releases): **`vjs`** (V8) and **`vjs-lite`** (QuickJS). Each archive includes the binary, `LICENSE.md`, and a profile-matched `types/` declaration directory.

## Prebuilt Release Binaries

Release archives are built for 6 targets across Linux, macOS, and Windows:

| Target Slug | Rust Target Triple | `vjs` Archive | `vjs-lite` Archive |
| --- | --- | --- | --- |
| `linux-amd64` | `x86_64-unknown-linux-gnu` | `vjs-<tag>-linux-amd64.tar.gz` | `vjs-lite-<tag>-linux-amd64.tar.gz` |
| `linux-arm64` | `aarch64-unknown-linux-gnu` | `vjs-<tag>-linux-arm64.tar.gz` | `vjs-lite-<tag>-linux-arm64.tar.gz` |
| `macos-arm64` | `aarch64-apple-darwin` | `vjs-<tag>-macos-arm64.tar.gz` | `vjs-lite-<tag>-macos-arm64.tar.gz` |
| `macos-amd64` | `x86_64-apple-darwin` | `vjs-<tag>-macos-amd64.tar.gz` | `vjs-lite-<tag>-macos-amd64.tar.gz` |
| `windows-amd64` | `x86_64-pc-windows-msvc` | `vjs-<tag>-windows-amd64.zip` | `vjs-lite-<tag>-windows-amd64.zip` |
| `windows-arm64` | `aarch64-pc-windows-msvc` | `vjs-<tag>-windows-arm64.zip` | `vjs-lite-<tag>-windows-arm64.zip` |

## Verify Checksums (`SHA256SUMS.txt`)

Every GitHub Release includes `SHA256SUMS.txt` covering all 12 archives. Always verify the download before extracting it onto your `PATH`.

```sh
# Example for v0.0.0-alpha.0 on Linux amd64
curl -LO https://github.com/neotales/vaeljs/releases/download/v0.0.0-alpha.0/vjs-v0.0.0-alpha.0-linux-amd64.tar.gz
curl -LO https://github.com/neotales/vaeljs/releases/download/v0.0.0-alpha.0/SHA256SUMS.txt

sha256sum --ignore-missing -c SHA256SUMS.txt
```

On macOS, use `shasum -a 256 -c SHA256SUMS.txt` (filtering to the downloaded archive) or `Get-FileHash` on Windows PowerShell.

## Archive Layout & Bundled `types/`

The release archives contain `vjs` (or `vjs-lite`), `LICENSE.md`, and `types/` directly at the archive root without a wrapping directory, so extract into a dedicated directory:

```sh
mkdir -p vjs && tar -xzf vjs-<tag>-linux-amd64.tar.gz -C vjs
```

```text
vjs/
├── vjs           # (or vjs.exe on Windows; vjs-lite / vjs-lite.exe for Lite)
├── LICENSE.md
└── types/
    ├── index.d.ts
    ├── globals.d.ts
    ├── vael/*.d.ts
    └── node/*.d.ts
```

Place `vjs` (or `vjs-lite`) on your `PATH`, and copy `types/` into your project or regenerate it anytime with `vjs types --out-dir ./types`.

## Symlink `argv[0]` Behavior

Both `vjs` and `vjs-lite` read their invoked binary name from `argv[0]` (stripping a trailing `.exe` on Windows). If you symlink `vjs` to another name on your `PATH`, `--help`, usage examples, and error prefixes adapt automatically:

```sh
ln -s /usr/local/bin/vjs /usr/local/bin/vael
vael --help
# Usage:
#   vael <file>                    run one module
#   ...
```

If `argv[0]` is empty or contains whitespace or control characters, the CLI falls back to `vjs` or `vjs-lite`.

## Building from Source

Building from source requires Rust **`1.100.0`** (pinned in `Cargo.toml` and `eng/size/toolchain`):

```sh
# Build the V8 runtime (default workspace)
cargo build --release --locked -p vaeljs --bin vjs

# Build the QuickJS Lite runtime (opt-in package)
cargo build --release --locked -p vaeljs-lite --bin vjs-lite

# Or install locally to ~/.cargo/bin
cargo install --locked --path vaeljs
cargo install --locked --path vaeljs-lite
```

The default `cargo build` builds only the V8 workspace crates so QuickJS (`rquickjs`) never enters the default dependency graph unless requested with `-p`.
