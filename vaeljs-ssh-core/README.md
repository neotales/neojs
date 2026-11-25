# vaeljs-ssh-core

Engine-neutral synchronous SSH command-execution core for the `vael:ssh` module,
designed to be shared by both the V8 runtime (`vaeljs-core`) and the QuickJS
runtime (`vaeljs-lite-core`).

This crate has zero JavaScript engine dependencies (`deno_core`, `deno_ast`,
`v8`, and `rquickjs` are absent) and zero `tokio` dependencies. Its direct
runtime dependencies are `neotales-ssh`, `thiserror`, and `zeroize` only.

```sh
cargo test --locked -p vaeljs-ssh-core
cargo clippy --locked -p vaeljs-ssh-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p vaeljs-ssh-core --no-deps
cargo tree --locked -p vaeljs-ssh-core -e normal
```

## Module surface

* `SPECIFIER`: `"vael:ssh"`
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
  agent. Non-Unix platforms reject agent authentication with `VAEL_SSH_AGENT`.
* **Bounded deadlines and sizes**: `connect_timeout` (default 15 s, max 120 s)
  and `exec_timeout` (default 30 s, max 600 s) bound TCP dialing, socket I/O
  inactivity, and `exec` session loops. Inputs and captured stdout are bounded
  (`MAX_COMMAND_BYTES` = 64 KiB, `MAX_OUTPUT_BYTES` = 8 MiB,
  `MAX_OPEN_CONNECTIONS` = 1024).
* **Fail-closed handle lifecycle**: Any `exec` failure after borrowing a
  connection closes and drops the underlying transport so partially read
  sessions cannot be reused.
* **Typed error codes**: [`SshError`] and [`SshErrorCode`] are
  `#[non_exhaustive]` and map every failure to the stable `VAEL_SSH_*` codes:
  `VAEL_SSH_ARGUMENT`, `VAEL_SSH_HOST_KEY_REQUIRED`, `VAEL_SSH_HOST_KEY`,
  `VAEL_SSH_KNOWN_HOSTS`, `VAEL_SSH_AUTH_METHOD`, `VAEL_SSH_CREDENTIAL`,
  `VAEL_SSH_AGENT`, `VAEL_SSH_CONNECT`, `VAEL_SSH_TIMEOUT`, `VAEL_SSH_HANDSHAKE`,
  `VAEL_SSH_AUTH`, `VAEL_SSH_SESSION`, `VAEL_SSH_LIMIT`, `VAEL_SSH_CLOSED`,
  `VAEL_SSH_IO`, and `VAEL_SSH_INTERNAL`.
