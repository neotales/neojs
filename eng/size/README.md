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
| Source | A git commit, `source.kind` `tag` or `commit`, and the exact source state fingerprint. |
| Canonical | `source.git_canonical` is true only for a clean tree. A dirty tree is refused unless `--allow-dirty` is passed, and then the record is explicitly non-canonical. |

The measurement is deliberately not one of these:

- the installed size after a package manager strips or compresses it;
- the sum of the workspace's Rust code, which would hide V8 or QuickJS;
- a peak RSS, a compressed archive, or an installed `.deb`/`.rpm` size;
- a "typical" number averaged across machines.

## Comparison model

There are two identifiers on every record:

- **`series_id`** — the strict comparison series. It covers every documented
  control that can move the byte count, but not the source commit and not the
  measured bytes. Two records with the same `series_id` are directly
  comparable.
- **`family_id`** — the intended longitudinal artifact: binary, target,
  profile, requested features. The trend report compares records in one family
  even when a control changed, and labels the delta.

### Strict series controls

- `artifact.package`, `artifact.bin`, `artifact.kind`;
- `platform.target` and `platform.host`;
- `toolchain.requested`, `rustc`, `cargo`, `llvm`, `cc`, `cxx`;
- `build.profile`;
- `build.requested_features` and `build.requested_no_default_features`;
- `build.effective_features` and `build.effective_linker` **status**. The
  current implementation does not capture effective values; it records
  `"status": "not-captured"` rather than claiming a feature set or linker it
  did not observe;
- `build.offline` and `build.locked`;
- `build.cargo_lock_sha256`;
- `build.workspace_path_sha256` — the build path, hashed. Rust embeds source
  paths in some strings, so two worktrees can differ by a few bytes even at the
  same commit;
- `build.cargo_config` — the discovered Cargo config files, by scope and
  content hash only. A `not-captured` status means the record predates this
  capture or the value was not observed; it is not treated as "no config";
- `build.env_overrides` — artifact-affecting variables the operator explicitly
  allowed, with values;
- `build.target_specific_env` — `absent`, `captured` or `not-captured`, with
  variable names;
- `build.v8_archive_status` and, when supplied, the archive name and
  decompressed content SHA-256.

`source.git_commit` is the variable a time series is supposed to move. The
source state is recorded separately so a dirty tree is never confused with a
clean commit. If a control changes, the trend report says which one.

### Baseline honesty

The first two records predate the schema-2 control capture. They are retained
because their bytes and source commit are real, but they are explicitly
relabelled:

- `source.state_status: "not-captured"`, `state_sha256: null`;
- `build.cargo_config.status: "not-captured"`;
- `build.target_specific_env.status: "not-captured"`;
- `build.effective_features.status: "not-captured"`;
- `build.effective_linker.status: "not-captured"`;
- the V8 archive is `supplied-provenance-unknown`, `authenticated: false`.

They therefore form their own strict series. The trend report labels the
control differences when it compares them with a later, fully captured record.
No missing control was invented.

### Canonical series

The pinned default is the toolchain in [`toolchain`](toolchain), currently
`1.100.0`, the same floor recorded in `Cargo.toml`. The canonical local command
is:

```sh
eng/size/measure.sh --target x86_64-unknown-linux-gnu
```

with a clean tree. That uses `cargo +1.100.0`, `--release --locked --offline`,
and a dedicated target directory at `target/size/`.

The default refuses a dirty tree because a dirty record is not attributable to
the commit alone. `--allow-dirty` records it with
`source.git_canonical: false`, and the report shows `canonical no`.

The script fingerprints the source state before and after each build. The
fingerprint covers the status, the tracked diff (including binary content) and
the contents of untracked files, with the ledger excluded. If the commit or that
fingerprint changes while a build runs, the measurement is refused: a binary
that cannot be attributed to the captured state is not recorded.

For a claimed **release** measurement, check out the release tag, verify the
tree is clean, and record the tag. A measurement from `dev` is a commit
measurement and is labelled that way; it is not a published release.

## Usage

```sh
# Show what would run. No build, no ledger write.
eng/size/measure.sh --dry-run

# Build, measure, print compact JSONL records, but do not append them.
# Progress goes to stderr, so stdout can be piped to jq.
eng/size/measure.sh --no-record

# Build, measure and append (the default).
eng/size/measure.sh

# One artifact, an explicit target and feature set.
eng/size/measure.sh --package neojs-lite \
    --target x86_64-unknown-linux-gnu \
    --features some,feature

# A non-canonical measurement from a dirty tree, explicitly labelled.
eng/size/measure.sh --allow-dirty

# Latest per strict series, longitudinal deltas, or raw history.
eng/size/report.sh
eng/size/report.sh --trend
eng/size/report.sh --all
```

A relative `--target-dir` or `--ledger` is resolved against the caller's
working directory and then normalized to an absolute path.

Exit status: `0` success, `1` configuration/build/validation error, `2` usage
error, `3` a requested artifact is missing.

Re-running an identical measurement is a no-op: `measure.sh` derives the record
key from the artifact, source state and all build controls, and exits `0` after
reporting `already recorded`.

### Build serialization

Builds sharing a target directory are serialized with an inode lock at
`<target-dir>/.size-build.lock`. A second measurement waits rather than compiling
into the same directory concurrently. The lock is in the dedicated target
directory and is never committed.

### Missing artifacts

`neojs-lite` is present at the base commit this tracker was added to, so the
first baseline covers both binaries. A branch that removes it (or builds a
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

### Build controls and unknowns

- **Requested, not effective, features.** `requested_features` and
  `requested_no_default_features` are what the command asked for. Effective
  features and the effective linker are labelled `not-captured`; no claim is
  made about them.
- **Cargo config.** Files Cargo would read from the workspace, its ancestors and
  `CARGO_HOME` are recorded by scope and content hash. Paths are not recorded.
  A missing file list with `status: captured` means none were found.
- **Target-specific environment.** `CARGO_TARGET_*` (except `CARGO_TARGET_DIR`,
  which this tool owns) and `CARGO_PROFILE_*`/`CARGO_BUILD_*` variables are
  refused unless `--allow-env-overrides` is passed, in which case their values
  are recorded. The same applies to `RUSTFLAGS`, `RUSTDOCFLAGS`, compiler
  variables, `V8_FROM_SOURCE`, `GN_ARGS` and similar inputs.
- **V8 archive.** `RUSTY_V8_ARCHIVE` is recorded as
  `supplied-provenance-unknown` with
  `provenance: "operator-supplied-local-file"` and
  `authenticated: false`. The content hash says which bytes were supplied; it
  does **not** prove the archive is the official upstream artifact, that it was
  authenticated, or that the build independently consumed it. When no archive
  is supplied, the status is `not-supplied-not-captured` (V8 package) or
  `not-applicable` (non-V8 package).
- **Offline.** Cargo is invoked with `--offline` by default. That flag does not
  stop a build script from downloading its own inputs. `--online` is opt-in and
  recorded as `"offline": false`.

## Ledger

[`measurements.jsonl`](measurements.jsonl) is the durable history: one JSON
object per line, append-only. It is text, not a binary artifact. Nothing in it
is machine-specific beyond the platform and toolchain strings that are the
point of the record.

Example (abridged; real records have every field):

```json
{"schema":2,"status":"ok","artifact":{"package":"neojs","bin":"neojs","kind":"release-binary","path":"neojs"},"source":{"git_commit":"...","git_dirty":false,"git_canonical":true,"git_tags":[],"kind":"commit","state_status":"captured","state_sha256":"..."},"platform":{"target":"x86_64-unknown-linux-gnu","host":"x86_64-unknown-linux-gnu","os":"Linux","arch":"x86_64"},"toolchain":{"requested":"1.100.0","rustc":"rustc 1.100.0 (...)","cargo":"cargo 1.100.0 (...)","cc":"cc (GCC) 16.2.1"},"build":{"profile":"release","requested_features":[],"requested_no_default_features":false,"effective_features":{"status":"not-captured"},"effective_linker":{"status":"not-captured"},"offline":true,"locked":true,"cargo_lock_sha256":"...","workspace_path_sha256":"...","cargo_config":{"status":"captured","files":[{"scope":"cargo-home","sha256":"..."}]},"env_overrides":{},"target_specific_env":{"status":"absent","names":[]},"v8_archive_status":"supplied-provenance-unknown","v8_archive":{"name":"...","sha256":"...","archive_sha256":"...","provenance":"operator-supplied-local-file","authenticated":false}},"measurement":{"method":"file-length-bytes","bytes":12345678,"sha256":"..."},"missing_reason":null,"measured_at":"2026-11-22T00:00:00Z","notes":null,"series_id":"...","family_id":"...","key":"..."}
```

Rules:

- **Generated, not typed.** `measure.sh` is the only writer; `validate.sh`
  recomputes `key`, `series_id` and `family_id` from the record contents, so a
  hand edit fails the check.
- **Append-only.** Do not rewrite or delete history. A correction is a new
  record with a `note`; the report's latest-per-series view uses the newest.
  The schema-1 baseline was relabelled once, before this branch was handed off,
  because it predated the schema; no measured bytes, commit or timestamp moved.
- **No binary artifacts.** The ledger is metadata. The executable stays in
  `target/`, which is gitignored. Do not attach or commit the binary.
- **`key`** covers the artifact, source state, controls and measured digest. It
  is what makes the second identical run a no-op.
- **`series_id`** covers the controls but not the source commit or measurement.
  **`family_id`** covers the intended artifact family.
- **Conflicts are evidence, not poison.** If one strict series, commit and
  source state produce two different digests, the records are kept. Strict
  `validate.sh` fails; `validate.sh --allow-conflicts` and `report.sh` still
  work and mark the series `conflict yes`. `measure.sh` records a conflicting
  measurement with a warning instead of hiding or overwriting it.
- **The ledger is not a source change.** When the ledger is inside the
  repository, it is excluded from the `git_dirty`/state fingerprint; otherwise
  appending the first measurement would make every later run look dirty. Other
  modified or untracked files still mark the tree dirty.
- **A non-canonical record is not a release measurement.** It is still a real
  measurement of that tree, but the commit alone does not identify its
  contents.

Fields that are intentionally absent: absolute build paths, user names, host
names, credentials, and anything read from a secret store. `workspace_path_sha256`
is a one-way hash used only to avoid false deduplication.

## Checks

```sh
eng/size/check.sh
```

That runs:

- `test.sh` — shell syntax, schema acceptance and rejection, key/series/family
  tampering, duplicate keys, provenance labels, conflict modes, target-directory
  lock contention, relative target-dir normalization, dirty-tree refusal,
  source-state change during build, Cargo config capture, environment override
  refusal, missing package handling, dry-run behaviour, trend/report rendering,
  and idempotent append, all in a temporary directory;
- `validate.sh` — strict schema, integrity, unique keys and conflict detection
  for the committed ledger.

The suite does not touch the network or modify the committed ledger. Its
scratch files live under `target/size/tmp/`, not the shared system temporary
directory. A measurement run is a separate, deliberate action.

Lint the scripts separately:

```sh
shellcheck -x -P eng/size eng/size/*.sh
```

For a local drill that exercises the whole path without writing:

```sh
eng/size/measure.sh --dry-run      # command construction
eng/size/measure.sh --no-record    # real build and measurement, printed only
eng/size/validate.sh               # schema and integrity of the committed ledger
eng/size/report.sh --trend         # longitudinal deltas and control changes
```

## Platform requirements

The tooling is written for GNU/Linux:

- Bash 4+;
- GNU coreutils (`realpath`, `sha256sum`, `mktemp`, `wc`, `sort`, `uniq`);
- util-linux `flock`;
- `jq` 1.6 or newer;
- `gzip` when `RUSTY_V8_ARCHIVE` is used;
- `rustup`, Cargo and the pinned Rust toolchain;
- a C and C++ compiler for the C dependencies.

macOS/BSD need GNU versions of these tools; the scripts do not claim to run
there unchanged.

## Adding a platform or a package

- **Platform:** install the Rust target, then measure with `--target <triple>`.
  The target and host are recorded; do not compare it with the host series.
- **Package:** pass `--package <name>`; it must be a workspace member with
  exactly one binary target. Multiple binary targets fail rather than guessing.
  Requested features are recorded per measurement.

A new family should start with a baseline record from a clean tree, so the first
delta has a known origin.

## Failure modes this is designed around

- A green check that skipped Lite: missing artifacts are recorded and exit `3`.
- Two runs compiling into one target directory: an inode lock serializes them.
- A dirty tree presented as a canonical commit: refused, or explicitly
  `canonical no` with `--allow-dirty`.
- A build that changes the source state: detected by before/after fingerprints
  and refused.
- A number quoted without its target: the target is in every record and every
  report row.
- A toolchain upgrade masquerading as a size regression: the toolchain is
  pinned in `toolchain` and recorded; the trend names the change.
- A stale `Cargo.lock`, Cargo config, or ambient `RUSTFLAGS`: recorded if
  captured or refused if not allowed, never silently attributed.
- An archive hash mistaken for authentication: provenance is labelled
  `operator-supplied-local-file`, `authenticated: false`.
- Effective features or linker assumed from the command line: labelled
  `not-captured`.
- A duplicate entry after re-running: deduplicated by `key`.
- A conflicting rebuild poisoning the report: kept, marked, and still
  reportable under `--allow-conflicts`.
- A machine-specific binary in git: the ledger is JSONL and `target/` is
  ignored.
