---
title: Commands & Flags
description: Exact CLI reference for vjs and vjs-lite subcommands, flags, environment variables, exit codes, and argv[0] behavior.
---

Both **`vjs`** and **`vjs-lite`** execute ECMAScript modules (`.js`, `.mjs`, extensionless) and erasable TypeScript (`.ts`, `.mts`), run isolated test suites, and emit `.d.ts` declarations.

## Subcommands

### Run a Module

```sh
vjs [options] <file-or-url>
vjs-lite [options] <file>
```

- Use `--` before a filename that starts with `-` (e.g., `vjs -- -script.ts`).
- `vjs` treats `https://`, `http://`, `gh:`, and `github:` inputs as URLs (still gated by `--allow-remote`, with `http:` always rejected by the loader). `vjs-lite` accepts local file paths only.
- Calling `vjs` or `vjs-lite` with no arguments prints an error and exits with code `2`.

### Run Tests (`test`)

```sh
vjs test [options] <file-or-glob>...
vjs-lite test [options] <file-or-glob>...
```

- Requires at least one file path or case-sensitive shell-style glob; there is **no** default file discovery.
- Matching files are sorted and deduplicated, and **each file runs in a fresh runtime**.
- Options (`--config`, `--no-config`, `--no-node-compat`, and `vjs`'s `--allow-remote`/`--offline`/etc.) may appear **before or after** `test`.
- Use `--` to end option parsing before a pattern starting with `-`.
- A file that registers zero tests, leaves a pending `test("name")`, leaves an unhandled rejection, or leaves unsettled work fails with exit code `1`.

### Emit Declarations (`types`)

```sh
vjs types [--profile <v8|lite>] [--out-dir <dir>]
vjs-lite types [--profile <lite|v8>] [--out-dir <dir>]
```

- `--profile <v8|lite>`: selects which runtime surface to emit (`vjs` defaults to `v8`; `vjs-lite` defaults to `lite`).
- `--out-dir <dir>`: writes the multi-file `types/` directory (`index.d.ts`, `globals.d.ts`, `vael/*.d.ts`, `node/*.d.ts`) instead of printing a single bundled `.d.ts` to stdout.
- `--help` / `-h` after `types` prints `types`-specific help.
- `types` rejects `--config`, `--no-config`, `--no-node-compat`, `--allow-remote`, `--no-allow-remote`, `--no-npm`, `--frozen-lockfile`, and `--offline`.

## Options Reference

| Option | Runtimes | Behavior |
| --- | --- | --- |
| `--config <path>` / `--config=<path>` | Both | Load an explicit `vael.jsonc` manifest instead of upward discovery. Cannot combine with `--no-config`. |
| `--no-config` | Both | Disable automatic upward `vael.jsonc` discovery. |
| `--no-node-compat` | Both | Disable built-in `node:*` compatibility modules at runtime. |
| `--allow-remote` | `vjs` | Enable `https:`, `gh:`, `github:`, `jsr:`, and network `npm:` fetches. |
| `--no-allow-remote` | `vjs` | Force remote loading off, overriding `VAELJS_ALLOW_REMOTE`. |
| `--frozen-lockfile` | `vjs` | Refuse any `npm:` or `package.json` resolution not pinned in `vael.lock`. |
| `--offline` | `vjs` | Refuse network fetches for npm packages; resolve strictly from the verified CAS cache. |
| `--no-npm` | `vjs` | Disable `npm:` specifiers and `package.json` dependency/`#imports` resolution. |
| `-h`, `--help` | Both | Print help (alone, after `test`, after `types`, or after `--allow-remote` in `vjs`). |
| `-V`, `--version` | Both | Print `<name> <version>` (e.g., `vjs 0.0.0-alpha.0`). |
| `--` | Both | End option parsing so a file or glob starting with `-` can be passed. |

## Environment Variables

`vjs` checks one persistent environment setting:

| Variable | Accepted Values | Behavior |
| --- | --- | --- |
| `VAELJS_ALLOW_REMOTE` | `1`, `0`, `true`, `false` (case-insensitive) | Enables or disables remote loading when neither `--allow-remote` nor `--no-allow-remote` is passed. Any invalid value is rejected at startup (exit code `2`), even when an explicit flag overrides it. |

`vjs-lite` reads no `VAELJS_*` CLI settings.

## Exit Codes

Both `vjs` and `vjs-lite` use the same exit code contract:

| Code | Meaning |
| --- | --- |
| `0` | The entry module finished cleanly, or every selected test file passed. |
| `1` | Runtime failure: module compile/load/strip error, uncaught exception, unhandled rejection, non-converging job queue, failing/pending test, or a test file registering zero tests. |
| `2` | Startup failure: invalid CLI flags or `VAELJS_ALLOW_REMOTE`, missing/invalid explicit `--config`, unreadable or unsupported entry file, or a test glob that failed to expand. |

If the entry module exits via `node:process` (`process.exit(code)` or `process.exitCode`), the process exits with `code & 0xff`.

## `argv[0]` Symlink Behavior

Both binaries inspect `argv[0]` at startup, extract the basename, and strip a trailing `.exe` suffix on Windows. If you symlink `vjs` as `vael`, usage lines and error prefixes automatically use `vael:` instead of `vjs:`.
