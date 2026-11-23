# Binary size tracking

`neojs` and `neojs-lite` are different runtimes, and their binaries answer
different questions. A size number without the artifact, target, toolchain and
feature set beside it is not a measurement; it is a rumour that will be
compared with an unrelated number six months later.

This directory measures the release-profile command-line binaries, records one
JSON object per measurement in an append-only ledger, and checks the ledger
before anyone quotes it.

## What is measured

| Field | Meaning |
|---|---|
| Artifact | The `[[bin]]` target of a workspace package. Default packages: `neojs`, `neojs-lite`. |
| Profile | `--release` only. Debug builds are not tracked here. |
| Value | `st_size` of the executable Cargo wrote, in bytes. |
| Digest | SHA-256 of the same file. |
| Scope | On-disk executable, before stripping, compression, signing, packaging or installation. |
| Source | A git commit, with `source.kind` `tag` or `commit`; `source.git_dirty` says whether the tree was clean. |

The measurement is deliberately not one of these:

- the installed size after a package manager strips or compresses it;
- the sum of the workspace's Rust code, which would hide V8 or QuickJS;
- a peak RSS, a compressed archive, or an installed `.deb`/`.rpm` size;
- a "typical" number averaged across machines.

## What makes two numbers comparable

Only records with the same **series** are comparable. The ledger stores every
input that can move the byte count:

- `platform.target` and `platform.host` — a cross build is a different series;
- `toolchain.rustc`, `toolchain.cargo`, `toolchain.llvm`, and the C/C++
  compiler versions, because `neojs-lite` compiles QuickJS and `neojs` links
  V8;
- `build.features` and `build.no_default_features`;
- `build.v8_archive` — the prebuilt V8 archive name and SHA-256, when one is
  supplied;
- `build.cargo_lock_sha256` — the dependency graph;
- `build.workspace_path_sha256` — the build path, hashed. Rust embeds source
  paths in some strings, so two worktrees can differ by a few bytes even at the
  same commit. The hash keeps two paths from being deduplicated as the same
  measurement without putting a machine path in the ledger;
- `build.env_overrides` — artifact-affecting variables that the operator
  explicitly allowed.

`source.git_commit` is the variable a time series is supposed to move. Every
other input is a control. If a control changes, say so when comparing.

The first measurement in a series is the baseline. A later measurement is a
delta only when the series fields above match.

### Canonical series

The pinned default is the toolchain in [`toolchain`](toolchain), currently
`1.100.0`, the same floor recorded in `Cargo.toml`. The canonical local command
is:

```sh
eng/size/measure.sh --target x86_64-unknown-linux-gnu
```

with a clean tree. That uses `cargo +1.100.0`, `--release --locked --offline`,
and a dedicated target directory at `target/size/`.

For a claimed **release** measurement, check out the release tag, verify the
tree is clean, and record the tag. A measurement from `dev` is a commit
measurement and is labelled that way; it is not a published release.

## Usage

```sh
# Show what would run. No build, no ledger write.
eng/size/measure.sh --dry-run

# Build, measure, print the records, but do not append them.
eng/size/measure.sh --no-record

# Build, measure and append (the default).
eng/size/measure.sh

# One artifact, an explicit target and feature set.
eng/size/measure.sh --package neojs-lite \
    --target x86_64-unknown-linux-gnu \
    --features some,feature

# Latest table, or the full history.
eng/size/report.sh
eng/size/report.sh --all
```

Exit status: `0` success, `1` configuration/build/validation error, `2` usage
error, `3` a requested artifact is missing.

Re-running an identical measurement is a no-op: `measure.sh` derives the record
key from the artifact, source and all build controls, and exits `0` after
reporting `already recorded`.

### Missing artifacts

`neojs-lite` is present at the base commit this tracker was added to, so the
first ledger entry covers both binaries. A branch that removes it (or builds a
subset) is still handled explicitly:

- a package directory absent from the checkout is recorded with
  `"status": "missing"`, `"missing_reason": "package-not-in-workspace"`;
- a directory that exists but is not a workspace member is
  `not-a-workspace-member`;
- a package with no binary target is `no-bin-target`.

Missing artifacts make the run exit `3` so a green report cannot hide a skipped
binary. `--allow-missing` records the same explicit missing entry and exits `0`
only when the caller has said the absence is expected. A missing record has no
bytes and no digest; it is never a zero.

### Offline builds

Cargo is invoked with `--offline` by default. That flag does not stop a Cargo
build script from downloading its own inputs, notably the prebuilt V8 archive.
To keep a V8 build offline, set `RUSTY_V8_ARCHIVE` to a matching
`librusty_v8_*.a.gz` file. The tracker records the archive basename and SHA-256
under `build.v8_archive`; it does not trust the filename alone. `--online`
exists for a fresh CI cache, but it is opt-in and is recorded as
`"offline": false`.

## Ledger

[`measurements.jsonl`](measurements.jsonl) is the durable history: one JSON
object per line, append-only. It is text, not a binary artifact. Nothing in it
is machine-specific beyond the platform and toolchain strings that are the
point of the record.

Example (abridged; real records have every field):

```json
{"schema":1,"status":"ok","artifact":{"package":"neojs","bin":"neojs","kind":"release-binary","path":"neojs"},"source":{"git_commit":"...","git_dirty":false,"git_tags":[],"kind":"commit"},"platform":{"target":"x86_64-unknown-linux-gnu","host":"x86_64-unknown-linux-gnu","os":"Linux","arch":"x86_64"},"toolchain":{"requested":"1.100.0","rustc":"rustc 1.100.0 (...)","cargo":"cargo 1.100.0 (...)","cc":"cc (GCC) 15.2.1"},"build":{"profile":"release","features":[],"no_default_features":false,"offline":true,"locked":true,"cargo_lock_sha256":"...","workspace_path_sha256":"...","v8_archive":null,"env_overrides":{}},"measurement":{"method":"file-length-bytes","bytes":12345678,"sha256":"..."},"missing_reason":null,"measured_at":"2026-11-22T00:00:00Z","notes":null,"key":"...","context_key":"..."}
```

Rules:

- **Generated, not typed.** `measure.sh` is the only writer; `validate.sh`
  recomputes `key` and `context_key` from the record contents, so a hand edit
  fails the check.
- **Append-only.** Do not rewrite or delete history. A correction is a new
  record with a `note`; the report's latest-per-series view uses the newest.
- **No binary artifacts.** The ledger is metadata. The executable stays in
  `target/`, which is gitignored. Do not attach or commit the binary.
- **`key`** covers the artifact, source, controls and measured digest. It is
  what makes the second identical run a no-op.
- **`context_key`** covers the controls but not the measured digest. Two records
  with one context and different digests mean the build is not reproducing
  itself; the validator rejects that ledger until the cause is understood or a
  missing control is recorded.

Fields that are intentionally absent: absolute build paths, user names, host
names, credentials, and anything read from a secret store. `workspace_path_sha256`
is a one-way hash used only to avoid false deduplication.

## Checks

```sh
eng/size/check.sh
```

That runs:

- `test.sh` — shell syntax, schema acceptance and rejection, key tampering,
  duplicate keys, context drift, idempotent append, dry-run behaviour, missing
  package handling, and report rendering, all in a temporary directory;
- `validate.sh` — the committed ledger must parse, satisfy the schema, have
  unique keys and reproducible contexts.

The suite does not build a binary, touch the network, or modify the ledger.
A measurement run is a separate, deliberate action.

For a local drill that exercises the whole path without writing:

```sh
eng/size/measure.sh --dry-run      # command construction
eng/size/measure.sh --no-record    # real build and measurement, printed only
eng/size/validate.sh               # schema and integrity of the committed ledger
```

## Adding a platform or a package

- **Platform:** install the Rust target, then measure with `--target <triple>`.
  The target and host are recorded; do not compare it with the host series.
- **Package:** pass `--package <name>`; it must be a workspace member with
  exactly one binary target. Multiple binary targets fail rather than guessing.
  Features are recorded per measurement.

A new series should start with a baseline record from a clean tree, so the first
delta has a known origin.

## Failure modes this is designed around

- A green check that skipped Lite: missing artifacts are recorded and exit `3`.
- A number quoted without its target: the target is in every record and every
  report row.
- A toolchain upgrade masquerading as a size regression: the toolchain is
  pinned in `toolchain` and recorded.
- A stale `Cargo.lock` or ambient `RUSTFLAGS`: refused, not silently attributed.
- A duplicate entry after re-running: deduplicated by `key`.
- A build that changes size without changing a recorded input: flagged by
  `context_key` and rejected by the validator.
- A machine-specific binary in git: the ledger is JSONL and `target/` is
  ignored.
