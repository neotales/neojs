# neojs-ssh-core

Engine-neutral synchronous SSH command-execution core for the `neo:ssh` module,
designed to be shared by both the V8 runtime (`neojs-core`) and the QuickJS
runtime (`neojs-lite-core`).

This crate has zero JavaScript engine dependencies (`deno_core`, `deno_ast`,
`v8`, and `rquickjs` are absent) and zero `tokio` dependencies. Its direct
runtime dependencies are `neotales-ssh`, `thiserror`, and `zeroize` only.

```sh
cargo test --locked -p neojs-ssh-core
cargo clippy --locked -p neojs-ssh-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p neojs-ssh-core --no-deps
cargo tree --locked -p neojs-ssh-core -e normal
```

## Module surface

* `SPECIFIER`: `"neo:ssh"`
* `EXPORTS`: `connect`, `MAX_COMMAND_BYTES`, `MAX_OUTPUT_BYTES`,
  `DEFAULT_CONNECT_TIMEOUT_MS`, `DEFAULT_EXEC_TIMEOUT_MS`
* Core API: `ConnectOptions`, `AuthConfig`, `HostTrust`, `Connection`,
  `ExecOutput`, `SshState`, `SshError`, `SshErrorCode`, and `connect`

## Security and resource invariants

* **Mandatory host trust (zero TOFU)**: [`HostTrust`] requires exactly one of
  an explicit SSH-format Ed25519 host-key pin ([`HostTrust::host_key`]) or an
  explicit `known_hosts` document ([`HostTrust::known_hosts`]). There is no
  insecure fallback and no ambient `~/.ssh/known_hosts` lookup.
* **Explicit credentials and zeroization**: [`AuthConfig`] requires exactly one
  of a password (`Zeroizing<String>`), an in-memory private key
  (`Zeroizing<Vec<u8>>` in PKCS#8 DER/PEM or OpenSSH binary/PEM, with an
  optional `Zeroizing<String>` passphrase for OpenSSH containers only), or an
  explicit Unix-only `Agent` opt-in. Secrets are kept in `Zeroizing` buffers so
  every validation error return wipes Rust copies, and error messages never
  echo passwords, passphrases, key bytes, commands, stdout, or peer banners.
* **Agent isolation**: `AuthConfig::Agent` reads `SSH_AUTH_SOCK` only after the
  server's host key has been verified, tries at most `MAX_AGENT_IDENTITIES` (16)
  identities with at most two requests per identity, and never forwards the
  agent. Non-Unix platforms reject agent authentication with `NEO_SSH_AGENT`.
* **Bounded deadlines and sizes**: `connect_timeout` (default 15 s, max 120 s)
  and `exec_timeout` (default 30 s, max 600 s) bound TCP dialing, socket I/O
  inactivity, and `exec` session loops. Inputs and captured stdout are bounded
  (`MAX_COMMAND_BYTES` = 64 KiB, `MAX_OUTPUT_BYTES` = 8 MiB,
  `MAX_OPEN_CONNECTIONS` = 1024).
* **Fail-closed handle lifecycle**: Any `exec` failure after borrowing a
  connection closes and drops the underlying transport so partially read
  sessions cannot be reused.
* **Typed error codes**: [`SshError`] and [`SshErrorCode`] are
  `#[non_exhaustive]` and map every failure to the stable `NEO_SSH_*` codes:
  `NEO_SSH_ARGUMENT`, `NEO_SSH_HOST_KEY_REQUIRED`, `NEO_SSH_HOST_KEY`,
  `NEO_SSH_KNOWN_HOSTS`, `NEO_SSH_AUTH_METHOD`, `NEO_SSH_CREDENTIAL`,
  `NEO_SSH_AGENT`, `NEO_SSH_CONNECT`, `NEO_SSH_TIMEOUT`, `NEO_SSH_HANDSHAKE`,
  `NEO_SSH_AUTH`, `NEO_SSH_SESSION`, `NEO_SSH_LIMIT`, `NEO_SSH_CLOSED`,
  `NEO_SSH_IO`, and `NEO_SSH_INTERNAL`.
