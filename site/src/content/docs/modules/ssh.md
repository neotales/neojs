---
title: ssh
description: Synchronous SSH command execution with mandatory host-key verification and exactly one authentication method.
---

`ssh` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `neo:ssh`; the registry form is `Neo.getBuiltInModule("ssh")`.

This is a synchronous command-execution slice. It is not a shell, PTY, SCP, SFTP, port forwarder, or
agent-forwarding client. It does not read `ssh_config`, identity files, or `~/.ssh/known_hosts`.

## Exports

| Export | Value |
| --- | --- |
| `connect` | Dial, verify, authenticate, and return an `SshConnection` |
| `MAX_COMMAND_BYTES` | `65536` |
| `MAX_OUTPUT_BYTES` | `8388608` (8 MiB) |
| `DEFAULT_CONNECT_TIMEOUT_MS` | `15000` |
| `DEFAULT_EXEC_TIMEOUT_MS` | `30000` |

## `connect(options)`

```js
import { connect } from "neo:ssh";

// Shape only: supply a real host, a real known-hosts document or host-key blob,
// and real credentials. The documentation build does not run this example.
const connection = connect({
    host,
    username,
    knownHosts, // string: a known-hosts document
    privateKey, // Uint8Array: in-memory key material
    connectTimeoutMs: 15_000,
    execTimeoutMs: 30_000,
});

try {
    const { stdout, status } = connection.exec("uname -s");
    console.log(status, stdout.length);
} finally {
    connection.close();
}
```

`connect` takes exactly one options object. Unknown option names are rejected rather than ignored,
so a misspelled or invented option cannot silently weaken host verification or authentication.

### Host trust

Exactly one of these is required:

| Option | Type | Meaning |
| --- | --- | --- |
| `hostKey` | `Uint8Array` | A complete SSH `ssh-ed25519` public-key blob |
| `knownHosts` | `string` | The text of a known-hosts document |

There is no insecure default, no first-use acceptance, and no fallback to a file. A missing or
mismatched host key refuses to connect.

### Authentication

Exactly one of these is required:

| Option | Type | Meaning |
| --- | --- | --- |
| `password` | `string` | Password authentication |
| `privateKey` | `Uint8Array` | In-memory unencrypted PKCS#8 DER, PKCS#8 PEM, binary `openssh-key-v1`, or OpenSSH PEM |
| `agent: true` | `boolean` | Explicit Unix SSH-agent opt-in |

`passphrase` is accepted only with `privateKey`, is used only for OpenSSH containers, and is wiped by
the SSH backend after parsing. Supplying one with a PKCS#8 key is refused. Encrypted PKCS#8 is not
supported.

Only `agent: true` reads `SSH_AUTH_SOCK`, and only after the host key is trusted. At most 16 agent
identities are tried, with at most two authentication requests per identity (32 total). Agent
authentication is Unix-only; Windows named pipes are rejected. There is no agent forwarding and no
implicit agent use.

### Other options

| Option | Default | Range |
| --- | --- | --- |
| `host` | required | Up to 1,024 bytes |
| `port` | 22 | Valid TCP port |
| `username` | required | Up to 256 bytes |
| `connectTimeoutMs` | 15,000 | 1 through 120,000 |
| `execTimeoutMs` | 30,000 | 1 through 600,000 |

`connectTimeoutMs` bounds each TCP connect attempt and is the socket inactivity timeout during the
blocking handshake and authentication. **DNS resolution is outside it.** The handshake is one
library call without an incremental deadline, so a peer that stalls fails, but one that trickles
bytes can extend it.

`execTimeoutMs` is checked between session events and before their socket I/O for each later
`exec`.

The call blocks the isolate until the handshake and authentication finish. It returns no Promise
and takes no callback.

## The connection

`connect` returns an `SshConnection`. User code cannot construct one directly.

| Member | Behaviour |
| --- | --- |
| `closed` | Whether the connection can no longer run a command |
| `exec(command)` | Run one command; returns `{ stdout, status }`; stderr is discarded |
| `close()` | Idempotent close; drops the native connection and TCP socket |

`exec` accepts a string encoded as UTF-8 or a `Uint8Array` of exact bytes. It blocks until the exit
status arrives and the session channel closes, or until `execTimeoutMs` expires. Captured stdout is
bounded by `MAX_OUTPUT_BYTES`; a command is bounded by `MAX_COMMAND_BYTES`. A failure after the
connection is engaged closes it. Argument validation, including a command over
`MAX_COMMAND_BYTES`, does not close it; inspect `closed` after catching.

`close()` does not send a graceful SSH disconnect. A later `exec` throws `NEO_SSH_CLOSED`.

## Errors

`connect` and `exec` throw `TypeError` for wrong argument counts or types and `RangeError` for a
number outside its documented range. Other failures are `Error` objects with
`name = "SshError"` and a stable own `code`:

| Code | Meaning |
| --- | --- |
| `NEO_SSH_HOST_KEY_REQUIRED` | Not exactly one of `hostKey` / `knownHosts` |
| `NEO_SSH_HOST_KEY` | The pinned host key was rejected |
| `NEO_SSH_KNOWN_HOSTS` | The known-hosts text is invalid |
| `NEO_SSH_AUTH_METHOD` | Not exactly one of `password` / `privateKey` / `agent: true` |
| `NEO_SSH_CREDENTIAL` | A credential was rejected or invalid |
| `NEO_SSH_AGENT` | The opted-in agent is unavailable, empty, oversized, or rejected |
| `NEO_SSH_CONNECT` | TCP connection failed |
| `NEO_SSH_TIMEOUT` | A documented timeout expired |
| `NEO_SSH_HANDSHAKE` | SSH handshake failed |
| `NEO_SSH_AUTH` | Authentication failed |
| `NEO_SSH_LIMIT` | A command, output, or input exceeded a documented bound |
| `NEO_SSH_IO` | Socket I/O failed during `exec` |
| `NEO_SSH_SESSION` | Session or channel failure during `exec` |
| `NEO_SSH_CLOSED` | `exec` called on a closed connection |
| `NEO_SSH_INTERNAL` | Unexpected backend failure |

Messages never contain a password, passphrase, key byte, command, or output byte.

## What it does not add

The module adds no filesystem, process, stdin, or permission capability. It performs DNS and
outbound TCP, completes the SSH handshake, verifies the configured host trust, authenticates with
exactly one method, and runs one command at a time. It has no unknown-host mode and reads no identity
file or default known-hosts file.

The agent timeout is not a hard wall clock: `UnixStream::connect` may block before any timeout is
installed, and the backend's 30-second read/write timeout is inactivity-based, so a trickling peer
can prolong enumeration or a signing request.
