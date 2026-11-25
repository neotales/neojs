/**
 * Synchronous SSH command execution with mandatory host-key verification.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:ssh" {
  /**
   * Maximum command byte length accepted by `SshConnection.exec` (64 KiB).
   *
   * @since 0.0.0-alpha.0
   */
  export const MAX_COMMAND_BYTES: 65536;

  /**
   * Maximum captured stdout byte length returned by `SshConnection.exec` (8 MiB).
   *
   * @since 0.0.0-alpha.0
   */
  export const MAX_OUTPUT_BYTES: 8388608;

  /**
   * Default TCP/handshake connect timeout in milliseconds (`15_000`).
   *
   * @since 0.0.0-alpha.0
   */
  export const DEFAULT_CONNECT_TIMEOUT_MS: 15000;

  /**
   * Default per-command execution timeout in milliseconds (`30_000`).
   *
   * @since 0.0.0-alpha.0
   */
  export const DEFAULT_EXEC_TIMEOUT_MS: 30000;

  /**
   * Error codes reported on `SshError.code` (`name = "SshError"`).
   *
   * @since 0.0.0-alpha.0
   */
  export type SshErrorCode =
    | "VAEL_SSH_ARGUMENT"
    | "VAEL_SSH_HOST_KEY_REQUIRED"
    | "VAEL_SSH_HOST_KEY"
    | "VAEL_SSH_KNOWN_HOSTS"
    | "VAEL_SSH_AUTH_METHOD"
    | "VAEL_SSH_CREDENTIAL"
    | "VAEL_SSH_AGENT"
    | "VAEL_SSH_CONNECT"
    | "VAEL_SSH_TIMEOUT"
    | "VAEL_SSH_HANDSHAKE"
    | "VAEL_SSH_AUTH"
    | "VAEL_SSH_SESSION"
    | "VAEL_SSH_LIMIT"
    | "VAEL_SSH_CLOSED"
    | "VAEL_SSH_IO"
    | "VAEL_SSH_INTERNAL";

  /**
   * Mandatory host-trust pin: supply **exactly one** of `hostKey` or `knownHosts`.
   *
   * @since 0.0.0-alpha.0
   */
  export type SshHostTrustOptions =
    | {
        /** Explicit SSH-format Ed25519 host public-key blob. */
        hostKey: Uint8Array;
        knownHosts?: undefined;
      }
    | {
        hostKey?: undefined;
        /** In-memory `known_hosts` host-key records. */
        knownHosts: string;
      };

  /**
   * Authentication method: supply **exactly one** of `password`, `privateKey`,
   * or `agent: true`.
   *
   * @since 0.0.0-alpha.0
   */
  export type SshAuthOptions =
    | {
        /** Password credential. */
        password: string;
        privateKey?: undefined;
        passphrase?: undefined;
        agent?: false;
      }
    | {
        /**
         * In-memory unencrypted PKCS#8 DER/PEM or OpenSSH (`openssh-key-v1`/PEM)
         * private key bytes.
         */
        privateKey: Uint8Array;
        /** Optional passphrase for encrypted `openssh-key-v1` containers only. */
        passphrase?: string;
        password?: undefined;
        agent?: false;
      }
    | {
        /**
         * Explicit Unix-only opt-in to authenticate via `SSH_AUTH_SOCK` after
         * host-key verification (tries up to 16 identities).
         */
        agent: true;
        password?: undefined;
        privateKey?: undefined;
        passphrase?: undefined;
      };

  /**
   * Options accepted by `connect(options)`. Unknown keys throw `TypeError`.
   *
   * @since 0.0.0-alpha.0
   */
  export type SshConnectOptions = {
    /** Remote hostname or IP address. */
    host: string;
    /** Remote TCP port (`1..=65535`, default `22`). */
    port?: number;
    /** SSH username (max 256 bytes). */
    username: string;
    /** Connect/handshake inactivity timeout in ms (`1..=120_000`, default `15_000`). */
    connectTimeoutMs?: number;
    /** Per-command execution timeout in ms (`1..=600_000`, default `30_000`). */
    execTimeoutMs?: number;
  } & SshHostTrustOptions &
    SshAuthOptions;

  /**
   * Result returned by `SshConnection.exec(command)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SshExecResult {
    /** Captured remote stdout bytes (at most `MAX_OUTPUT_BYTES`). Remote stderr is discarded. @since 0.0.0-alpha.0 */
    stdout: Uint8Array;
    /** Remote exit status code. @since 0.0.0-alpha.0 */
    status: number;
  }

  /**
   * Synchronous, authenticated SSH command-execution connection returned by
   * `connect(options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SshConnection {
    /** Whether this connection is closed and can no longer run commands. @since 0.0.0-alpha.0 */
    readonly closed: boolean;

    /**
     * Synchronously execute a single command on the remote host.
     *
     * Blocks until exit status arrives and the session channel closes, or until
     * `execTimeoutMs` expires. Fatal transport/session/timeout/output-limit
     * errors close the connection; pre-flight validation errors leave it open.
     *
     * @param command UTF-8 string or raw bytes (max `MAX_COMMAND_BYTES`).
     * @throws {TypeError} on wrong argument type or arity.
     * @throws {Error} (`name = "SshError"`) with `code` `"VAEL_SSH_CLOSED"`,
     * `"VAEL_SSH_LIMIT"`, `"VAEL_SSH_TIMEOUT"`, `"VAEL_SSH_IO"`,
     * `"VAEL_SSH_SESSION"`, or `"VAEL_SSH_INTERNAL"`.
     * @since 0.0.0-alpha.0
     */
    exec(command: string | Uint8Array): SshExecResult;

    /**
     * Close the underlying socket. Idempotent; does not send an SSH disconnect
     * packet. Subsequent `exec` calls throw `VAEL_SSH_CLOSED`.
     *
     * @since 0.0.0-alpha.0
     */
    close(): void;
  }

  /**
   * Synchronously dial, verify host identity, authenticate, and return an
   * `SshConnection`.
   *
   * Never reads `ssh_config`, identity files, or `~/.ssh/known_hosts`.
   *
   * @throws {TypeError} on invalid options or unknown option keys.
   * @throws {RangeError} on out-of-range port or timeout values.
   * @throws {Error} (`name = "SshError"`) with `code: SshErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function connect(options: SshConnectOptions): SshConnection;
}
