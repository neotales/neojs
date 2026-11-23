---
title: Remote loading and security
description: What --allow-remote enables, how the HTTPS cache behaves, and the security properties it does not provide.
---

Remote loading is a V8 runtime feature. It is **off by default** and is not implemented in NeoJS
Lite.

## Enabling it

Per command:

```sh
neojs --allow-remote https://example.com/main.js
neojs --allow-remote main.js
neojs test --allow-remote "tests/**/*.test.js"
```

Persistently for an environment:

```sh
NEOJS_ALLOW_REMOTE=1 neojs main.js
```

`NEOJS_ALLOW_REMOTE=1` or `true` (case-insensitive) makes remote loading the default for every
`neojs` run and `neojs test` in that environment. `0` or `false` keeps it off. Any other value,
including empty, is a startup error with exit 2. An explicit `--allow-remote` or
`--no-allow-remote` overrides a valid setting; an invalid value is still rejected.

:::danger
This is a security-relevant persistent switch. It lets every module and test file in that
environment load and run remote code with the same authority as local code. Set it deliberately, not
in a shared shell profile.
:::

## What is accepted

- Only credential-free `https:` URLs.
- A remote entry, or an absolute `https:` import from a local module.
- Relative imports inside a remote module resolve against that module's URL and stay remote.
- `.js`, `.mjs`, `.ts`, and `.mts` work, statically and dynamically. TypeScript is type-stripped as
  it is locally.
- A remote module cannot import a `file:` URL. There is no implicit transition from remote code to
  the local filesystem.
- `http:`, `data:`, `blob:`, `node:`, `npm:`, and `jsr:` are rejected.

## Redirects

When a redirect is followed, the final URL is the module's identity and the requested URL is an
alias for it, as in a browser:

- `import.meta.url` is the final URL.
- Relative imports resolve against it.
- A later import of either spelling reaches the same module instance.
- Two URLs that differ only by query are two module instances.

Each redirect is checked: the target must be credential-free HTTPS and within the redirect limit, so
a downgrade to HTTP fails before the target is requested. Cross-origin HTTPS redirects are allowed
because NeoJS sends no cookie, authorization header, or referrer, and reads no proxy settings from
the environment. A URL with a username or password is rejected rather than forwarded.

## The package cache

Downloaded modules are cached automatically in one user-profile package cache:

```text
$XDG_CACHE_HOME/neojs/packages
$HOME/.cache/neojs/packages   # when XDG_CACHE_HOME is unset
```

The cache key is a hash of the canonical URL including its query, so secrets in a query do not
appear as file names, and two inputs that differ by query are separate entries. Entries are written
atomically, so concurrent processes cannot publish a half-written module, and each entry records and
verifies its body length and SHA-256 digest.

Freshness and revalidation:

- A fresh `Cache-Control: max-age` response is reused without a request. Freshness includes the
  upstream `Age` and `Date`, so a response that was already old when it arrived is stale
  immediately.
- A stale entry is revalidated with `If-None-Match`/`If-Modified-Since`, and a `304` reuses the
  cached body.
- `no-store` does not store the body; it leaves an absent-entry tombstone so an older `200` cannot
  resurrect it.
- A response without a freshness lifetime is revalidated on the next run.
- Publications are serialized per key and use persistent monotonic revision files. A `200` loses to
  a newer entry or `no-store` tombstone, and a `304` uses its own request revision rather than a
  later completion token, so it cannot outrank a later-started `200` or `no-store`.
- Representation identity combines the validator, when present, with the actual body digest, so a
  weak ETag shared by different bytes is not treated as the same representation.
- Lock waits are capped at 250 ms and the remaining fetch deadline. Header and metadata lines are
  bounded, one file handle is used per read, the body length is checked before allocation, and
  appended bytes are rejected as a cache miss.

:::caution
The cache has no signature or content-addressed store. It is not integrity protection against
someone who can write it and does not pin a version. `RemoteOptions::with_cache_dir` and
`without_cache` control the directory.
:::

## Resource bounds

The runtime enforces a per-module body limit, an aggregate byte limit, a remote module count limit,
a request limit, a redirect limit, and a per-module deadline. A module and its bytes are admitted
before the network or cache is consulted; a body is read only up to the remaining aggregate
allowance; a rejected module is neither fetched nor cached. Each distinct requested URL consumes a
module slot even when its fetch fails. A separate request limit counts every network request,
including retries and each redirect hop. Idle connection pooling is disabled.

The deadline is per module, not a wall-clock bound on the whole program. These are **resource
bounds, not a sandbox, CPU bound, or V8 heap bound**.

## What remote loading is not

Remote loading is a capability switch, not a permission system.

- It is **not SSRF protection**: there is no host allowlist, private-address rejection, or DNS
  pinning. NeoJS fetches whatever HTTPS URL the program names with the process's network authority.
- Remote code runs with the same authority as local code and is not sandboxed.
- There is no permission model to restrict filesystem, network, or process access.

## Error redaction limits

Generated V8 errors redact known remote module URLs from stack-frame locations, but only from
messages whose error name and prefix match a generated diagnostic shape. The module's identity and
`import.meta.url` stay the final URL.

A script-supplied exception message, including a quoted relative URL-like string, text shaped like
"Cannot find module", or ` at <URL>:`, and a custom stack are preserved, so code can still expose a
URL it puts in its own error. A user-thrown `SyntaxError`/`TypeError` that exactly copies a generated
prefix and location shape is indistinguishable from V8's own text and may be redacted.

**Do not use module specifiers as secret storage.**

## Configuration surface

The public Rust surface is `RemoteOptions` in `neojs-core/src/remote.rs`. The CLI exposes the two
flags described above. See the [V8 runtime](/runtime/) page for embedding.

## Related

- [Command line](/cli/) — flag placement and exit codes.
- [Limits and unsupported features](/limits/) — what is not implemented.
