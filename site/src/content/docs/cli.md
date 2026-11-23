---
title: Command line
description: The neojs command, remote-loading flags, the NEOJS_ALLOW_REMOTE switch, test selection, and exit codes.
sidebar:
  order: 2
---

The `neojs` command is the V8 runtime's command-line wrapper. It runs one module, or runs one or
more test files, each in a fresh runtime.

## Help text

`neojs --help` starts with `neojs <version>` and then prints this summary. The rest of the output
describes modules, remote loading, the test runner, and exit codes; the help text is not a
machine-readable format.

```text
Usage:
  neojs <file>                    run one module
  neojs --allow-remote <file-or-https-url>
  neojs --no-allow-remote <file>
  neojs test <file-or-glob>...    run test files
  neojs test --allow-remote <file-or-glob>...
  neojs test --no-allow-remote <file-or-glob>...
  neojs --allow-remote test <file-or-glob>...

Options:
  --allow-remote   allow https: entry and imports; off by default
  --no-allow-remote
                   force remote loading off, overriding NEOJS_ALLOW_REMOTE
  -h, --help       show this help; alone, after `test`, or after `--allow-remote`
  -V, --version    show the version; alone, or after `--allow-remote`
  --               end option parsing, for a file or pattern beginning with `-`

Environment:
  NEOJS_ALLOW_REMOTE  1 or true enables remote loading, 0 or false keeps it off
                      (case-insensitive); an explicit flag overrides a valid
                      setting, and an invalid value is rejected at startup
```

With no arguments, `neojs` exits 2 and points at `--help`.

## Run a module

```sh
neojs hello.js
neojs ./src/main.ts
```

Remote loading is off by default. To run an `https:` entry, or let a local module import an
absolute `https:` URL:

```sh
neojs --allow-remote https://example.com/main.js
neojs --allow-remote main.js
```

`--allow-remote` may appear before the module, or before or after `test`. `--no-allow-remote`
forces remote loading off even when `NEOJS_ALLOW_REMOTE` would enable it.

:::caution
Remote code runs with the same authority as local code. Remote loading is a capability switch, not
a permission system or a sandbox. It is also not SSRF protection: there is no host allowlist,
private-address rejection, or DNS pinning. Read [Remote loading and
security](/remote-security/) before enabling it in an environment.
:::

## Persistent remote opt-in

`--allow-remote` is a per-command choice. `NEOJS_ALLOW_REMOTE` changes the default for every
`neojs` run and every `neojs test` in that environment:

| Value | Effect |
| --- | --- |
| `1`, `true` (case-insensitive) | Remote loading is the default |
| `0`, `false` (case-insensitive) | Remote loading stays off |
| Anything else, including empty | Startup error, exit 2 |

An explicit `--allow-remote` or `--no-allow-remote` overrides a valid setting. An invalid value is
rejected even when an explicit flag is present, so a typo cannot hide.

:::danger
This is a security-relevant persistent switch. It lets every module and test file in that
environment load and run remote code with the same authority as local code. Set it deliberately,
not in a shared shell profile.
:::

## Run tests

```sh
neojs test "tests/**/*.test.js"
neojs test tests/unit.test.js tests/smoke.test.ts
neojs --allow-remote test "tests/**/*.test.js"
neojs test --allow-remote "tests/**/*.test.js"
neojs test --no-allow-remote "tests/**/*.test.js"
```

The command expands shell-style globs deterministically, sorts the selected files by canonical
path, removes duplicates, and runs each file in a fresh runtime. A literal file, a malformed glob,
and a glob that selects no regular file are all errors. There is no default discovery pattern, no
watch mode, no parallelism, no filtering, no retries, and no subtests, hooks, mocks, or snapshots.

`--` ends option parsing, so a file literally named `--allow-remote` can still be selected:

```sh
neojs test -- --allow-remote.test.js
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | The module finished, or every selected test file ran with no failures |
| `1` | The module failed to compile, resolve imports, link, or run; or a test failed, was pending, left an unhandled rejection or unsettled event-loop work, or the file registered no tests |
| `2` | NeoJS could not start: bad arguments, an unreadable or unsupported entry file, or a test file or glob that could not be expanded |

## Runtime limits at the command line

There is no timeout or interrupt. A JavaScript loop that never exits hangs the process. A `Runtime`
runs one entry module; the command creates a fresh runtime for each test file. See [Limits and
unsupported features](/limits/).
