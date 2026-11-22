// The `ssh` built-in module as seen by `import { connect } from "ssh"` or
// `import { connect } from "neo:ssh"`.
//
// The implementation lives in `neo.js`, which the runtime evaluates once per
// realm, and calls private host ops registered by `neojs-core/src/ssh.rs`.
// This file only re-exports the public values, so every named export is the
// same value as the matching property of `Neo.getBuiltInModule("ssh")`. Nothing
// here reaches the private module registry: it reads the public module object
// and binds its properties.
//
// The exported list must match the properties of the `ssh` object in `neo.js`.
// `neojs-core/tests/ssh.rs` asserts that the two lists are identical and that
// each export is identity-equal to the registry property.
//
// This is the synchronous command-execution slice only. It is not a shell, PTY,
// SCP, SFTP, port forwarder, or agent-forwarding client, and it does not read
// `ssh_config`, identity files, or `~/.ssh/known_hosts`. Host trust is
// mandatory: `connect` requires exactly one of `hostKey` (an Ed25519 SSH key
// blob as a Uint8Array) or `knownHosts` (the text of a known-hosts document).
// Authentication is exactly one of a password, an in-memory private key, or an
// explicit Unix `agent: true`; only that opt-in reads `SSH_AUTH_SOCK`, and only
// after the host key is trusted. There is no insecure default and no
// asynchronous or callback form.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace `connect` or a
// constant. Adding a property to the exported function is visible through both
// paths because they share the same function.
const ssh = Neo.getBuiltInModule("ssh");

export const connect = ssh.connect;
export const MAX_COMMAND_BYTES = ssh.MAX_COMMAND_BYTES;
export const MAX_OUTPUT_BYTES = ssh.MAX_OUTPUT_BYTES;
export const DEFAULT_CONNECT_TIMEOUT_MS = ssh.DEFAULT_CONNECT_TIMEOUT_MS;
export const DEFAULT_EXEC_TIMEOUT_MS = ssh.DEFAULT_EXEC_TIMEOUT_MS;
