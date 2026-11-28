/**
 * Synchronous argv-only subprocess execution, PATH executable lookup, and OS
 * advisory file locking (`vjs-lite` profile).
 *
 * Available in the `vjs-lite` (`vaeljs-lite`) runtime. Exposes the synchronous
 * exports from `vael:exec` (`Command`, `CommandError`, `FileLock`,
 * `NotFoundOnPathError`, `Output`, `cmd`, `execSync`, `lockFileSync`,
 * `runSync`, `whichAllSync`, `whichSync`); async worker APIs and streaming
 * `ChildProcess` handles are only available in `vjs` (`vaeljs`).
 *
 * Never invokes a shell (`sh -c` or `cmd.exe /c`). All program names,
 * arguments, working directories, and environment entries are validated against
 * NUL bytes and invalid structure before spawning.
 *
 * @example
 * ```ts
 * import { cmd, execSync, lockFileSync, whichSync } from "vael:exec";
 *
 * const sh = whichSync("sh");
 * if (sh !== null) {
 *   const out = execSync([sh, "-c", "echo hello"]).validate();
 *   console.log(out.text().trim());
 * }
 *
 * const built = cmd("echo").arg("world").execSync();
 * console.log(built.lines());
 *
 * using lock = lockFileSync("./app.lock");
 * console.log(lock.mode);
 * ```
 *
 * @since 0.0.0-alpha.2
 */
declare module "vael:exec" {
  /**
   * Stable machine-readable `VAEL_EXEC_*` error codes exposed on `Error.code`
   * by `vael:exec` operations.
   *
   * @since 0.0.0-alpha.2
   */
  export type ExecErrorCode =
    | "VAEL_EXEC_NOT_FOUND"
    | "VAEL_EXEC_PERMISSION_DENIED"
    | "VAEL_EXEC_INVALID_ARGUMENT"
    | "VAEL_EXEC_TIMEOUT"
    | "VAEL_EXEC_OUTPUT_TOO_LARGE"
    | "VAEL_EXEC_UNSUPPORTED_SIGNAL"
    | "VAEL_EXEC_LOCK_WOULD_BLOCK"
    | "VAEL_EXEC_LOCK_TIMEOUT"
    | "VAEL_EXEC_NON_ZERO"
    | "VAEL_EXEC_STDERR"
    | "VAEL_EXEC_IO";

  /**
   * Standard stream wiring mode for subprocess `stdin`, `stdout`, and `stderr`.
   *
   * - `"inherit"`: inherit the parent descriptor
   * - `"piped"`: connect an OS pipe
   * - `"null"`: redirect to `/dev/null` or `NUL`
   *
   * @since 0.0.0-alpha.2
   */
  export type StdioMode = "inherit" | "piped" | "null";

  /**
   * Explicit subprocess termination or control signal.
   *
   * @since 0.0.0-alpha.2
   */
  export type Signal =
    | "SIGINT"
    | "SIGTERM"
    | "SIGKILL"
    | "SIGHUP"
    | "SIGQUIT"
    | "SIGUSR1"
    | "SIGUSR2";

  /**
   * Advisory file lock sharing mode.
   *
   * - `"shared"`: concurrent shared reader lock
   * - `"exclusive"`: exclusive writer lock blocking other shared and exclusive locks
   *
   * @since 0.0.0-alpha.2
   */
  export type LockMode = "shared" | "exclusive";

  /**
   * Options accepted by `Command`, `cmd`, `execSync`, and `runSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface CommandOptions {
    /**
     * Additional literal argv arguments appended after any arguments passed in
     * the command invocation (never shell-expanded).
     *
     * @since 0.0.0-alpha.2
     */
    args?: readonly string[];

    /**
     * Working directory for the subprocess (inherits parent cwd when omitted).
     *
     * @since 0.0.0-alpha.2
     */
    cwd?: string;

    /**
     * Environment variables set on the subprocess (`undefined` entries are
     * omitted or removed).
     *
     * @since 0.0.0-alpha.2
     */
    env?: Record<string, string | undefined>;

    /**
     * Environment variable names to remove from the subprocess environment.
     *
     * @since 0.0.0-alpha.2
     */
    envRemove?: readonly string[];

    /**
     * Clear all inherited environment variables before applying `env`
     * (default `false`).
     *
     * @since 0.0.0-alpha.2
     */
    clearEnv?: boolean;

    /**
     * Standard input configuration: `"inherit"`, `"piped"`, `"null"`, or
     * fixed bytes (`string` or `Uint8Array`) written to stdin and closed at EOF.
     *
     * @since 0.0.0-alpha.2
     */
    stdin?: StdioMode | string | Uint8Array;

    /**
     * Standard output wiring mode (`"inherit"`, `"piped"`, or `"null"`).
     *
     * @since 0.0.0-alpha.2
     */
    stdout?: StdioMode;

    /**
     * Standard error wiring mode (`"inherit"`, `"piped"`, or `"null"`).
     *
     * @since 0.0.0-alpha.2
     */
    stderr?: StdioMode;

    /**
     * Finite positive execution timeout in milliseconds. On expiry, sends
     * `killSignal` and escalates to `"SIGKILL"` after `killGrace` / `killGraceMs`.
     *
     * @since 0.0.0-alpha.2
     */
    timeout?: number;

    /**
     * Alias for `timeout` (mutually exclusive with `timeout`).
     *
     * @since 0.0.0-alpha.2
     */
    timeoutMs?: number;

    /**
     * Initial termination signal sent when `timeout` expires (default `"SIGTERM"`).
     *
     * @since 0.0.0-alpha.2
     */
    killSignal?: Signal;

    /**
     * Grace period in milliseconds after `killSignal` before escalating to
     * `"SIGKILL"` (default `2_000`; `0` escalates immediately).
     *
     * @since 0.0.0-alpha.2
     */
    killGrace?: number;

    /**
     * Alias for `killGrace` (mutually exclusive with `killGrace`).
     *
     * @since 0.0.0-alpha.2
     */
    killGraceMs?: number;

    /**
     * Maximum bytes captured per output stream (`stdout` and `stderr`,
     * default `16_777_216` / 16 MiB, hard cap `67_108_864` / 64 MiB).
     *
     * @since 0.0.0-alpha.2
     */
    maxOutputBytes?: number;
  }

  /**
   * Subprocess exit status metadata returned by `runSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface CommandStatus {
    /**
     * Exit status code (`128 + signo` if killed by a signal on Unix, `-1` if unavailable).
     *
     * @since 0.0.0-alpha.2
     */
    readonly code: number;

    /**
     * Termination signal name (for example `"SIGTERM"` or `"SIGKILL"`), or `null`
     * when the subprocess exited normally.
     *
     * @since 0.0.0-alpha.2
     */
    readonly signal: string | null;

    /**
     * `true` when the subprocess exited with status `0` and did not time out.
     *
     * @since 0.0.0-alpha.2
     */
    readonly success: boolean;

    /**
     * `true` when execution exceeded `timeout` and the subprocess was terminated.
     *
     * @since 0.0.0-alpha.2
     */
    readonly timedOut: boolean;
  }

  /**
   * Options accepted by `lockFileSync`.
   *
   * `nonBlocking: true` and `timeout` are mutually exclusive and throw with
   * `"VAEL_EXEC_INVALID_ARGUMENT"` if combined.
   *
   * @since 0.0.0-alpha.2
   */
  export interface LockOptions {
    /**
     * Advisory lock sharing mode (default `"exclusive"`).
     *
     * @since 0.0.0-alpha.2
     */
    mode?: LockMode;

    /**
     * Create the target lock file if it does not already exist (default `true`).
     *
     * @since 0.0.0-alpha.2
     */
    create?: boolean;

    /**
     * Fail immediately with `"VAEL_EXEC_LOCK_WOULD_BLOCK"` if a conflicting
     * lock is held (default `false`).
     *
     * @since 0.0.0-alpha.2
     */
    nonBlocking?: boolean;

    /**
     * Optional bounded wait duration in milliseconds before failing with
     * `"VAEL_EXEC_LOCK_TIMEOUT"`.
     *
     * @since 0.0.0-alpha.2
     */
    timeout?: number;
  }

  /**
   * Optional metadata attached to a `CommandError`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface CommandErrorOptions {
    /** Exit status code from the failed subprocess. @since 0.0.0-alpha.2 */
    readonly exitCode?: number;
    /** Termination signal name, or `null`. @since 0.0.0-alpha.2 */
    readonly signal?: Signal | string | null;
    /** Whether the failure resulted from exceeding `timeout`. @since 0.0.0-alpha.2 */
    readonly timedOut?: boolean;
    /** Captured `Output` from the failed command, when available. @since 0.0.0-alpha.2 */
    readonly output?: Output;
    /** Target executable name/path, when applicable. @since 0.0.0-alpha.2 */
    readonly fileName?: string;
  }

  /**
   * Error thrown by `Output.validate()` or `vael:exec` operations when a
   * subprocess exits with a non-zero status code, signal termination, timeout,
   * or execution error.
   *
   * @since 0.0.0-alpha.2
   */
  export class CommandError extends Error {
    /**
     * Error name (`"CommandError"`).
     *
     * @since 0.0.0-alpha.2
     */
    override name: "CommandError";

    /**
     * Machine-readable error code.
     *
     * @since 0.0.0-alpha.2
     */
    readonly code: ExecErrorCode | string;

    /**
     * Exit status code from the failed subprocess, when available.
     *
     * @since 0.0.0-alpha.2
     */
    readonly exitCode?: number;

    /**
     * Termination signal name, or `null` if not killed by a signal.
     *
     * @since 0.0.0-alpha.2
     */
    readonly signal?: Signal | string | null;

    /**
     * `true` when the failure resulted from exceeding `timeout`.
     *
     * @since 0.0.0-alpha.2
     */
    readonly timedOut: boolean;

    /**
     * Captured `Output` object from the failed subprocess, when available.
     *
     * @since 0.0.0-alpha.2
     */
    readonly output?: Output;

    /**
     * Target executable or path associated with the error, when available.
     *
     * @since 0.0.0-alpha.2
     */
    readonly fileName?: string;

    /**
     * Construct a `CommandError`.
     *
     * @param code Machine-readable `ExecErrorCode` or custom code string.
     * @param message Human-readable error message.
     * @param options Optional failure metadata.
     * @since 0.0.0-alpha.2
     */
    constructor(code: ExecErrorCode | string, message: string, options?: CommandErrorOptions);
  }

  /**
   * Error thrown when a target executable cannot be found on `PATH` or at an
   * explicit path (`code: "VAEL_EXEC_NOT_FOUND"`).
   *
   * @since 0.0.0-alpha.2
   */
  export class NotFoundOnPathError extends Error {
    /**
     * Error name (`"NotFoundOnPathError"`).
     *
     * @since 0.0.0-alpha.2
     */
    override name: "NotFoundOnPathError";

    /**
     * Stable error code (`"VAEL_EXEC_NOT_FOUND"`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly code: "VAEL_EXEC_NOT_FOUND";

    /**
     * Program name or path that failed lookup.
     *
     * @since 0.0.0-alpha.2
     */
    readonly fileName: string;

    /**
     * Construct a `NotFoundOnPathError`.
     *
     * @param fileName Program name or path that was not found.
     * @param message Optional custom human-readable error message.
     * @since 0.0.0-alpha.2
     */
    constructor(fileName: string, message?: string);
  }

  /**
   * Captured output and termination metadata returned by `execSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export class Output implements CommandStatus {
    /**
     * Exit status code (`128 + signo` if terminated by signal on Unix, `-1` if unavailable).
     *
     * @since 0.0.0-alpha.2
     */
    readonly code: number;

    /**
     * Termination signal name (such as `"SIGTERM"` or `"SIGKILL"`), or `null`.
     *
     * @since 0.0.0-alpha.2
     */
    readonly signal: string | null;

    /**
     * `true` when the subprocess exited with code `0` and did not time out.
     *
     * @since 0.0.0-alpha.2
     */
    readonly success: boolean;

    /**
     * `true` when execution exceeded `timeout` and the subprocess was terminated.
     *
     * @since 0.0.0-alpha.2
     */
    readonly timedOut: boolean;

    /**
     * `true` when captured output was truncated at a byte boundary.
     *
     * @since 0.0.0-alpha.2
     */
    readonly truncated: boolean;

    /**
     * Captured standard output bytes.
     *
     * @since 0.0.0-alpha.2
     */
    readonly stdout: Uint8Array;

    /**
     * Captured standard error bytes.
     *
     * @since 0.0.0-alpha.2
     */
    readonly stderr: Uint8Array;

    /**
     * Decode `stdout` as UTF-8 (replacing invalid sequences with `U+FFFD`).
     *
     * @returns Decoded standard output string.
     * @since 0.0.0-alpha.2
     */
    text(): string;

    /**
     * Split decoded `stdout` into lines, stripping `\n` and `\r\n` line endings.
     *
     * @returns Array of standard output lines.
     * @since 0.0.0-alpha.2
     */
    lines(): string[];

    /**
     * Decode `stdout` as UTF-8 and parse it as JSON.
     *
     * @returns Parsed JSON value.
     * @since 0.0.0-alpha.2
     */
    json<T = unknown>(): T;

    /**
     * Decode `stderr` as UTF-8 (replacing invalid sequences with `U+FFFD`).
     *
     * @returns Decoded standard error string.
     * @since 0.0.0-alpha.2
     */
    errorText(): string;

    /**
     * Split decoded `stderr` into lines, stripping `\n` and `\r\n` line endings.
     *
     * @returns Array of standard error lines.
     * @since 0.0.0-alpha.2
     */
    errorLines(): string[];

    /**
     * Decode `stderr` as UTF-8 and parse it as JSON.
     *
     * @returns Parsed JSON value.
     * @since 0.0.0-alpha.2
     */
    errorJson<T = unknown>(): T;

    /**
     * Assert that the subprocess succeeded (`this.success === true`, or
     * `predicate(this.code)` returns `true`), returning `this` on success or
     * throwing `CommandError` on failure.
     *
     * @param predicate Optional exit-code predicate `(code: number) => boolean`.
     * @param failOnStderr If `true`, throw `CommandError` (`"VAEL_EXEC_STDERR"`) when `stderr` is non-empty.
     * @returns `this` when validation passes.
     * @throws {CommandError} when validation fails.
     * @example
     * ```ts
     * import { execSync } from "vael:exec";
     *
     * const text = execSync(["echo", "ok"]).validate().text();
     * console.log(text);
     * ```
     * @since 0.0.0-alpha.2
     */
    validate(predicate?: (code: number) => boolean, failOnStderr?: boolean): this;
  }

  /**
   * Argv-only fluent command specification builder.
   *
   * Never invokes a shell; every argument, path, and environment entry is
   * validated against NUL bytes and invalid structure before spawning.
   *
   * @since 0.0.0-alpha.2
   */
  export class Command {
    /**
     * Construct a `Command` specification for `program`, `(program, args, options?)`, or `(argv, options?)`.
     *
     * @param program Executable name/path or non-empty `[program, ...args]` array.
     * @param argsOrOptions Literal argument array or command options.
     * @param options Optional command configuration when `argsOrOptions` is an argument array.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) when `program` is empty or contains NUL bytes.
     * @since 0.0.0-alpha.2
     */
    constructor(program: string, options?: CommandOptions);
    constructor(program: string, args: readonly string[], options?: CommandOptions);
    constructor(argv: readonly string[], options?: CommandOptions);

    /**
     * Append a single literal argument (never shell-expanded).
     *
     * @param value Literal argument string (must not contain NUL bytes).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `value` contains a NUL byte.
     * @since 0.0.0-alpha.2
     */
    arg(value: string): this;

    /**
     * Append multiple literal arguments in order (never shell-expanded).
     *
     * @param values Array of literal argument strings.
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if any argument contains a NUL byte.
     * @since 0.0.0-alpha.2
     */
    args(values: readonly string[]): this;

    /**
     * Set the subprocess working directory.
     *
     * @param path Non-empty NUL-free directory path.
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `path` is empty or contains NUL bytes.
     * @since 0.0.0-alpha.2
     */
    cwd(path: string): this;

    /**
     * Set or override a single environment variable on the subprocess.
     *
     * @param key Non-empty environment variable name (no `'='` or NUL).
     * @param value Environment variable value (no NUL).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) on invalid key or NUL-containing value.
     * @since 0.0.0-alpha.2
     */
    env(key: string, value: string): this;

    /**
     * Set multiple environment variables on the subprocess.
     *
     * @param vars Plain object map (`undefined` values are skipped).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) on invalid keys or NUL-containing values.
     * @since 0.0.0-alpha.2
     */
    envs(vars: Record<string, string | undefined>): this;

    /**
     * Remove an inherited or previously configured environment variable.
     *
     * @param key Non-empty environment variable name (no `'='` or NUL).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) on invalid key.
     * @since 0.0.0-alpha.2
     */
    envRemove(key: string): this;

    /**
     * Clear all inherited and previously configured environment variables.
     *
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    envClear(): this;

    /**
     * Configure standard input wiring (`"inherit"`, `"piped"`, `"null"`, or
     * fixed bytes written before closing EOF).
     *
     * @param mode Standard input mode or byte payload.
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    stdin(mode: StdioMode | string | Uint8Array): this;

    /**
     * Configure standard output wiring (`"inherit"`, `"piped"`, or `"null"`).
     *
     * @param mode Standard output wiring mode.
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    stdout(mode: StdioMode): this;

    /**
     * Configure standard error wiring (`"inherit"`, `"piped"`, or `"null"`).
     *
     * @param mode Standard error wiring mode.
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    stderr(mode: StdioMode): this;

    /**
     * Set a finite positive execution timeout in milliseconds.
     *
     * @param ms Timeout duration in milliseconds (`> 0`).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `ms <= 0` or non-finite.
     * @since 0.0.0-alpha.2
     */
    timeout(ms: number): this;

    /**
     * Set the initial signal sent when `timeout` expires (default `"SIGTERM"`).
     *
     * @param signal Signal name.
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    killSignal(signal: Signal): this;

    /**
     * Set the grace period in milliseconds after `killSignal` before escalating
     * to `"SIGKILL"` (default `2_000`; `0` escalates immediately).
     *
     * @param ms Grace duration in milliseconds (`>= 0`).
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    killGrace(ms: number): this;

    /**
     * Set the maximum captured byte limit per output stream (default 16 MiB,
     * hard cap 64 MiB).
     *
     * @param bytes Maximum bytes per stream (`0..=67_108_864`).
     * @returns `this` for fluent chaining.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `bytes` exceeds 64 MiB.
     * @since 0.0.0-alpha.2
     */
    maxOutputBytes(bytes: number): this;

    /**
     * Return the full `[program, ...args]` argv vector configured on this
     * command.
     *
     * @returns Copy of the `[program, ...args]` argument array.
     * @since 0.0.0-alpha.2
     */
    toArgs(): string[];

    /**
     * Execute this command synchronously, capturing `stdout` and `stderr`.
     *
     * @returns Captured `Output`.
     * @throws {NotFoundOnPathError} if the executable cannot be found.
     * @throws {Error} with `code: ExecErrorCode` on spawn, timeout, or output-limit failure.
     * @since 0.0.0-alpha.2
     */
    outputSync(): Output;

    /**
     * Alias for `outputSync()`.
     *
     * @returns Captured `Output`.
     * @since 0.0.0-alpha.2
     */
    execSync(): Output;

    /**
     * Execute this command synchronously with inherited standard streams by default.
     *
     * @returns `CommandStatus`.
     * @throws {NotFoundOnPathError} if the executable cannot be found.
     * @throws {Error} with `code: ExecErrorCode` on spawn or timeout failure.
     * @since 0.0.0-alpha.2
     */
    statusSync(): CommandStatus;

    /**
     * Alias for `statusSync()`.
     *
     * @returns `CommandStatus`.
     * @since 0.0.0-alpha.2
     */
    runSync(): CommandStatus;
  }

  /**
   * Active OS advisory file lock guard returned by `lockFileSync`.
   *
   * Releases the underlying lock when `unlock()` is called or when disposed via
   * `using` (`[Symbol.dispose]()`).
   *
   * @since 0.0.0-alpha.2
   */
  export class FileLock {
    /**
     * Target lock file path associated with this guard.
     *
     * @since 0.0.0-alpha.2
     */
    readonly path: string;

    /**
     * Lock sharing mode held by this guard (`"shared"` or `"exclusive"`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly mode: LockMode;

    /**
     * `true` while the advisory OS lock is currently held by this guard.
     *
     * @since 0.0.0-alpha.2
     */
    readonly locked: boolean;

    /**
     * Explicitly unlock and close the underlying file handle. Idempotent on
     * subsequent calls after release.
     *
     * @throws {Error} (`"VAEL_EXEC_IO"`) if the OS unlock operation fails.
     * @since 0.0.0-alpha.2
     */
    unlock(): void;

    /**
     * Release the advisory file lock when exiting a `using` scope.
     *
     * @since 0.0.0-alpha.2
     */
    [Symbol.dispose](): void;
  }

  /**
   * Create a fluent `Command` builder without executing it.
   *
   * @param program Executable name/path or `[program, ...args]` array.
   * @param argsOrOptions Optional literal argument array or command configuration.
   * @param options Optional command configuration when `argsOrOptions` is an argument array.
   * @returns Configured `Command` builder.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `program` is empty or contains NUL bytes.
   * @example
   * ```ts
   * import { cmd } from "vael:exec";
   *
   * const command = cmd("env").envClear().env("FOO", "bar");
   * console.log(command.toArgs());
   * ```
   * @since 0.0.0-alpha.2
   */
  export function cmd(program: string, options?: CommandOptions): Command;
  export function cmd(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): Command;
  export function cmd(argv: readonly string[], options?: CommandOptions): Command;

  /**
   * Execute a command synchronously and capture `stdout` and `stderr`
   * (defaulting `stdin` to `"null"` and `stdout`/`stderr` to `"piped"`).
   *
   * @param command Configured `Command` builder.
   * @returns Captured `Output`.
   * @throws {NotFoundOnPathError} if the executable cannot be found.
   * @throws {Error} with `code: ExecErrorCode` on invalid arguments, timeout, or output overflow.
   * @since 0.0.0-alpha.2
   */
  export function execSync(command: Command): Output;

  /**
   * Execute an `[program, ...args]` argv array synchronously and capture
   * `stdout` and `stderr`.
   *
   * @param argv Non-empty `[program, ...args]` array (never shell-expanded).
   * @param options Optional working directory, environment, stdio, and limits.
   * @returns Captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function execSync(argv: readonly string[], options?: CommandOptions): Output;

  /**
   * Execute `program` synchronously with optional `CommandOptions`.
   *
   * @param program Executable name or path.
   * @param options Optional arguments, working directory, environment, stdio, and limits.
   * @returns Captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function execSync(program: string, options?: CommandOptions): Output;

  /**
   * Execute `program` with `args` synchronously and capture `stdout` and `stderr`.
   *
   * @param program Executable name or path.
   * @param args Literal argv arguments.
   * @param options Optional working directory, environment, stdio, and limits.
   * @returns Captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function execSync(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): Output;

  /**
   * Execute a command synchronously with inherited standard streams by default
   * and return exit status metadata.
   *
   * @param command Configured `Command` builder.
   * @returns `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function runSync(command: Command): CommandStatus;

  /**
   * Execute an `[program, ...args]` argv array synchronously with inherited
   * standard streams by default.
   *
   * @param argv Non-empty `[program, ...args]` array.
   * @param options Optional command configuration.
   * @returns `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function runSync(argv: readonly string[], options?: CommandOptions): CommandStatus;

  /**
   * Execute `program` synchronously with inherited standard streams by default.
   *
   * @param program Executable name or path.
   * @param options Optional command configuration.
   * @returns `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function runSync(program: string, options?: CommandOptions): CommandStatus;

  /**
   * Execute `program` with `args` synchronously with inherited standard
   * streams by default.
   *
   * @param program Executable name or path.
   * @param args Literal argv arguments.
   * @param options Optional command configuration.
   * @returns `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function runSync(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): CommandStatus;

  /**
   * Synchronously find the first executable matching `name` on `PATH`
   * (optionally prepended by `prependPath`).
   *
   * @param name Bare executable name or explicit path.
   * @param prependPath Optional `PATH`-formatted prefix searched before `PATH`.
   * @returns Resolved executable path, or `null` if not found.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `name` is empty or contains NUL bytes.
   * @since 0.0.0-alpha.2
   */
  export function whichSync(
    name: string,
    prependPath?: string | { readonly prependPath?: string },
  ): string | null;

  /**
   * Synchronously find all executables matching `name` across `prependPath`
   * and `PATH` in search order.
   *
   * @param name Bare executable name or explicit path.
   * @param prependPath Optional `PATH`-formatted prefix or options object `{ prependPath? }`.
   * @returns Array of matching executable paths in search order.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `name` is empty or contains NUL bytes.
   * @since 0.0.0-alpha.2
   */
  export function whichAllSync(
    name: string,
    prependPath?: string | { readonly prependPath?: string },
  ): string[];

  /**
   * Synchronously acquire an OS advisory file lock on `path` (rejecting
   * symbolic links).
   *
   * @param path Target lock file path (must not be a symlink or end with `.`).
   * @param options Optional `{ mode, create, nonBlocking, timeout }` settings.
   * @returns Active `FileLock` guard.
   * @throws {Error} with `code` `"VAEL_EXEC_LOCK_WOULD_BLOCK"`, `"VAEL_EXEC_LOCK_TIMEOUT"`, `"VAEL_EXEC_INVALID_ARGUMENT"`, or `"VAEL_EXEC_IO"`.
   * @example
   * ```ts
   * import { lockFileSync } from "vael:exec";
   *
   * const lock = lockFileSync("./service.lock", { mode: "exclusive", nonBlocking: true });
   * try {
   *   // critical section
   * } finally {
   *   lock.unlock();
   * }
   * ```
   * @since 0.0.0-alpha.2
   */
  export function lockFileSync(path: string, options?: LockOptions): FileLock;
}
