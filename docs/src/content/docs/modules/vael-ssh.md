---
title: vael:ssh
description: Synchronous SSH-2 client with mandatory explicit host trust, bounded command outputs, and zero implicit config reads.
---

`vael:ssh` provides a synchronous SSH-2 command-execution client backed by `neotales-ssh`. It enforces explicit host-key pinning, never reads `ssh_config`, identity files, or `~/.ssh/known_hosts`, and bounds command and output sizes.

:::note[Availability]
`vael:ssh` is available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

## Exports

| Export | Value / Signature | Description |
| --- | --- | --- |
| `connect` | `(options: SshConnectOptions) => SshConnection` | Synchronously dial, verify host key, authenticate, and open a session. |
| `MAX_COMMAND_BYTES` | `65536` | Maximum command byte length accepted by `exec()` (`64 KiB`). |
| `MAX_OUTPUT_BYTES` | `8388608` | Maximum captured stdout byte length returned by `exec()` (`8 MiB`). |
| `DEFAULT_CONNECT_TIMEOUT_MS` | `15000` | Default TCP/handshake connect timeout in ms (`15s`). |
| `DEFAULT_EXEC_TIMEOUT_MS` | `30000` | Default per-command execution timeout in ms (`30s`). |

## Mandatory Host Trust & Authentication

`connect(options)` throws `TypeError` on unknown keys and `RangeError` on out-of-range `port` or timeout values. You must supply **exactly one** host-trust field and **exactly one** authentication method:

### Host Trust (`SshHostTrustOptions`)

- `hostKey: Uint8Array` — Explicit SSH-format Ed25519 host public-key blob.
- `knownHosts: string` — In-memory `known_hosts` host-key records.

### Authentication (`SshAuthOptions`)

- `password: string` — Password authentication.
- `privateKey: Uint8Array` (+ optional `passphrase?: string`) — In-memory unencrypted PKCS#8 DER/PEM or OpenSSH (`openssh-key-v1`/PEM) private key bytes (`passphrase` applies only to encrypted `openssh-key-v1` containers).
- `agent: true` — Explicit Unix-only opt-in to authenticate via `SSH_AUTH_SOCK` after host-key verification (tries up to 16 identities).

### Connection Options (`SshConnectOptions`)

| Option | Type | Default / Bounds |
| --- | --- | --- |
| `host` | `string` | Required hostname or IP address. |
| `port` | `number` | Default `22` (`1..=65535`). |
| `username` | `string` | Required SSH username (max `256` bytes). |
| `connectTimeoutMs` | `number` | Default `15_000` (`1..=120_000` ms). |
| `execTimeoutMs` | `number` | Default `30_000` (`1..=600_000` ms). |

## `SshConnection`

| Member | Signature | Behavior |
| --- | --- | --- |
| `closed` | `readonly boolean` | `true` once closed or invalidated by a fatal error. |
| `exec` | `(command: string \| Uint8Array) => SshExecResult` | Synchronously run `command` (max `MAX_COMMAND_BYTES`). Returns `{ stdout: Uint8Array, status: number }`. Remote stderr is discarded. |
| `close` | `() => void` | Close the underlying socket (idempotent; does not send an SSH disconnect packet). |

Fatal transport, session, timeout, or output-limit errors during `exec()` automatically close the connection; pre-flight validation errors leave it open. Calling `exec()` after `close()` throws `VAEL_SSH_CLOSED`.

## Error Codes (`SshErrorCode`)

Failures throw an `Error` with `name = "SshError"` and `code` set to one of:

| Code | Meaning |
| --- | --- |
| `"VAEL_SSH_ARGUMENT"` | Invalid host, username, or command argument. |
| `"VAEL_SSH_HOST_KEY_REQUIRED"` | Neither or both of `hostKey` / `knownHosts` supplied. |
| `"VAEL_SSH_HOST_KEY"` | Host key verification failed or key format invalid. |
| `"VAEL_SSH_KNOWN_HOSTS"` | Invalid or unmatched `knownHosts` entry. |
| `"VAEL_SSH_AUTH_METHOD"` | Missing or ambiguous authentication option. |
| `"VAEL_SSH_CREDENTIAL"` | Invalid private key or passphrase format. |
| `"VAEL_SSH_AGENT"` | `SSH_AUTH_SOCK` unavailable or agent error. |
| `"VAEL_SSH_CONNECT"` | TCP dial failed. |
| `"VAEL_SSH_TIMEOUT"` | Connect or command inactivity timeout expired. |
| `"VAEL_SSH_HANDSHAKE"` | SSH protocol handshake failed. |
| `"VAEL_SSH_AUTH"` | Server rejected authentication credentials. |
| `"VAEL_SSH_SESSION"` | SSH channel/session open or protocol failure. |
| `"VAEL_SSH_LIMIT"` | Command exceeded `MAX_COMMAND_BYTES` or stdout exceeded `MAX_OUTPUT_BYTES`. |
| `"VAEL_SSH_CLOSED"` | `exec()` called on a closed connection. |
| `"VAEL_SSH_IO"` | Underlying socket I/O failure. |
| `"VAEL_SSH_INTERNAL"` | Unexpected internal backend error. |

## Example

```ts
import { readFile, readTextFile } from "vael:fs";
import { connect } from "vael:ssh";

const conn = connect({
  host: "10.0.0.10",
  username: "deploy",
  knownHosts: readTextFile("./deploy_known_hosts"),
  privateKey: readFile("./deploy_ed25519"),
  connectTimeoutMs: 5000,
  execTimeoutMs: 15000,
});

try {
  const { stdout, status } = conn.exec("uname -a");
  console.log(status, new TextDecoder().decode(stdout));
} finally {
  conn.close();
}
```
