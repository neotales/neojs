/**
 * Argv-only subprocess execution, PATH executable lookup, and OS advisory file
 * locking.
 *
 * Available in the `vjs` (`vaeljs`) runtime (`exec`, `execSync`, `run`,
 * `runSync`, `spawn`, `which`, `whichSync`, `whichAll`, `whichAllSync`,
 * `lockFile`, `lockFileSync`, `cmd`, `Command`, `ChildProcess`, `Output`,
 * `CommandError`, `NotFoundOnPathError`, `FileLock`); `vjs-lite`
 * (`vaeljs-lite`) exposes the synchronous APIs only.
 *
 * Never invokes a shell (`sh -c` or `cmd.exe /c`). All program names,
 * arguments, working directories, and environment entries are validated against
 * NUL bytes and invalid structure before spawning.
 *
 * @example
 * ```ts
 * import { cmd, exec, execSync, lockFileSync, whichSync } from "vael:exec";
 *
 * const sh = whichSync("sh");
 * if (sh !== null) {
 *   const out = execSync([sh, "-c", "echo hello"]).validate();
 *   console.log(out.text().trim());
 * }
 *
 * const asyncOut = await exec(cmd("echo").arg("world"));
 * console.log(asyncOut.lines());
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
   * Options accepted by `Command`, `cmd`, `exec`, `execSync`, `run`, `runSync`,
   * and `spawn`.
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

    /**
     * Optional per-instance execution logging / audit hook invoked right before
     * spawning the command. Throwing from this hook aborts spawn.
     *
     * @since 0.0.0-alpha.2
     */
    log?: CommandLogHandler;
  }

  /**
   * Event metadata passed to a `CommandLogHandler` before a subprocess spawns.
   *
   * @since 0.0.0-alpha.2
   */
  export interface CommandLogEvent {
    /**
     * Resolved executable name or path about to be executed.
     *
     * @since 0.0.0-alpha.2
     */
    readonly file: string;

    /**
     * Frozen snapshot of argument strings passed to the executable (excluding `argv[0]`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly args: readonly string[];

    /**
     * Configured working directory for the command, if explicitly set.
     *
     * @since 0.0.0-alpha.2
     */
    readonly cwd?: string;

    /**
     * Execution entry point mode that triggered the spawn.
     *
     * @since 0.0.0-alpha.2
     */
    readonly mode: "exec" | "execSync" | "run" | "runSync" | "spawn";
  }

  /**
   * Pre-spawn command logging or audit callback.
   *
   * Throwing an error aborts execution before spawning the subprocess.
   *
   * @param file Resolved executable name or path.
   * @param args Frozen argument array (`argv.slice(1)`).
   * @param event Frozen null-prototype event metadata.
   * @since 0.0.0-alpha.2
   */
  export type CommandLogHandler = (
    file: string,
    args: readonly string[],
    event: CommandLogEvent,
  ) => void;

  /**
   * Disposable handle returned by `addLogger` and `Command.addLogger`.
   *
   * Calling `dispose()` or exiting a `using` block removes only this
   * subscription idempotently and flips `active` to `false`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface CommandLogSubscription {
    /**
     * `true` while the handler subscription remains registered and active.
     *
     * @since 0.0.0-alpha.2
     */
    readonly active: boolean;

    /**
     * Unregister this specific log handler subscription idempotently.
     *
     * @since 0.0.0-alpha.2
     */
    dispose(): void;

    /**
     * Dispose this subscription automatically when exiting a `using` block.
     *
     * @since 0.0.0-alpha.2
     */
    [Symbol.dispose](): void;
  }

  /**
   * Executable lookup entry stored in a `PathFinder` registry.
   *
   * @since 0.0.0-alpha.2
   */
  export interface PathFinderOptions {
    /**
     * Canonical executable name for this entry.
     *
     * @since 0.0.0-alpha.2
     */
    name: string;

    /**
     * Executable binary name or explicit path to look up instead of `name`.
     *
     * @since 0.0.0-alpha.2
     */
    executable?: string;

    /**
     * Environment variable name checked for an explicit executable path override.
     *
     * @since 0.0.0-alpha.2
     */
    envVariable?: string;

    /**
     * Cached resolved path returned when `noCache` is not `true`.
     *
     * @since 0.0.0-alpha.2
     */
    cached?: string;

    /**
     * Disable caching resolved paths on this entry when `true`.
     *
     * @since 0.0.0-alpha.2
     */
    noCache?: boolean;

    /**
     * Ordered candidate fallback paths on Windows (supports `${VAR}` and `%VAR%` expansion).
     *
     * @since 0.0.0-alpha.2
     */
    windows?: readonly string[];

    /**
     * Ordered candidate fallback paths on Linux (supports `${VAR}` and `%VAR%` expansion).
     *
     * @since 0.0.0-alpha.2
     */
    linux?: readonly string[];

    /**
     * Ordered candidate fallback paths on macOS (`darwin`; supports `${VAR}` and `%VAR%` expansion).
     *
     * @since 0.0.0-alpha.2
     */
    darwin?: readonly string[];
  }

  /**
   * Subprocess exit status metadata returned by `run`, `runSync`,
   * `ChildProcess.tryWait`, and `ChildProcess.wait`.
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
   * Options accepted by `lockFile` and `lockFileSync`.
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
   * Captured output and termination metadata returned by `exec`, `execSync`,
   * and `ChildProcess.waitWithOutput`.
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
     * Execute this command asynchronously, capturing `stdout` and `stderr`.
     *
     * Available in `vjs` (`vaeljs`) only.
     *
     * @returns Promise resolving to captured `Output`.
     * @throws {NotFoundOnPathError} if the executable cannot be found.
     * @throws {Error} with `code: ExecErrorCode` on spawn, timeout, or output-limit failure.
     * @since 0.0.0-alpha.2
     */
    output(): Promise<Output>;

    /**
     * Alias for `output()`.
     *
     * Available in `vjs` (`vaeljs`) only.
     *
     * @returns Promise resolving to captured `Output`.
     * @since 0.0.0-alpha.2
     */
    exec(): Promise<Output>;

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
     * Execute this command asynchronously with inherited standard streams by default.
     *
     * Available in `vjs` (`vaeljs`) only.
     *
     * @returns Promise resolving to `CommandStatus`.
     * @throws {NotFoundOnPathError} if the executable cannot be found.
     * @throws {Error} with `code: ExecErrorCode` on spawn or timeout failure.
     * @since 0.0.0-alpha.2
     */
    status(): Promise<CommandStatus>;

    /**
     * Alias for `status()`.
     *
     * Available in `vjs` (`vaeljs`) only.
     *
     * @returns Promise resolving to `CommandStatus`.
     * @since 0.0.0-alpha.2
     */
    run(): Promise<CommandStatus>;

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

    /**
     * Execute asynchronously, validate zero exit status, and decode `stdout` as UTF-8 text.
     *
     * @returns Promise resolving to decoded `stdout` text.
     * @since 0.0.0-alpha.2
     */
    text(): Promise<string>;

    /**
     * Execute asynchronously, validate zero exit status, and split `stdout` into lines.
     *
     * @returns Promise resolving to `stdout` lines.
     * @since 0.0.0-alpha.2
     */
    lines(): Promise<string[]>;

    /**
     * Execute asynchronously, validate zero exit status, and parse `stdout` as JSON.
     *
     * @returns Promise resolving to parsed JSON value.
     * @since 0.0.0-alpha.2
     */
    json<T = unknown>(): Promise<T>;

    /**
     * Set or clear (`null`) the single per-instance `log` hook on this command.
     *
     * @param handler Pre-spawn hook or `null` to clear.
     * @returns `this` for fluent chaining.
     * @throws {TypeError} if `handler` is neither a function nor `null`.
     * @since 0.0.0-alpha.2
     */
    log(handler: CommandLogHandler | null): this;

    /**
     * Attach a disposable per-instance log/audit handler to this command.
     *
     * @param handler Pre-spawn log/audit callback.
     * @returns Frozen `CommandLogSubscription` handle.
     * @throws {TypeError} if `handler` is not a function.
     * @since 0.0.0-alpha.2
     */
    addLogger(handler: CommandLogHandler): CommandLogSubscription;

    /**
     * Clear the per-instance `log` hook and all `addLogger` subscriptions on
     * this command, marking existing subscriptions inactive.
     *
     * @returns `this` for fluent chaining.
     * @since 0.0.0-alpha.2
     */
    clearLoggers(): this;

    /**
     * Spawn this command as a managed `ChildProcess` handle with piped streams.
     *
     * Available in `vjs` (`vaeljs`) only.
     *
     * @returns Spawned `ChildProcess` handle.
     * @throws {NotFoundOnPathError} if the executable cannot be found.
     * @throws {Error} with `code: ExecErrorCode` on spawn failure.
     * @since 0.0.0-alpha.2
     */
    spawn(): ChildProcess;
  }

  /**
   * Managed streaming subprocess handle returned by `spawn`.
   *
   * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
   *
   * @since 0.0.0-alpha.2
   */
  export class ChildProcess {
    /**
     * OS-assigned identifier for the spawned subprocess.
     *
     * @since 0.0.0-alpha.2
     */
    readonly pid: number;

    /**
     * Write UTF-8 text or raw bytes to the subprocess's piped `stdin` and flush.
     *
     * @param data Text or bytes to write to `stdin`.
     * @returns Promise resolving when the write and flush complete.
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"` or `"VAEL_EXEC_IO"`) if `stdin` is not piped or closed.
     * @since 0.0.0-alpha.2
     */
    writeStdin(data: string | Uint8Array): Promise<void>;

    /**
     * Close the parent's handle to `stdin` (sending EOF to the subprocess).
     *
     * @since 0.0.0-alpha.2
     */
    closeStdin(): void;

    /**
     * Read up to `maxBytes` currently available bytes from piped `stdout`.
     * Returns an empty `Uint8Array` on EOF.
     *
     * @param maxBytes Maximum bytes to read (capped at 64 MiB).
     * @returns Promise resolving to the read chunk (`Uint8Array`).
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"` or `"VAEL_EXEC_IO"`) if `stdout` is not piped.
     * @since 0.0.0-alpha.2
     */
    readStdout(maxBytes?: number): Promise<Uint8Array>;

    /**
     * Read up to `maxBytes` currently available bytes from piped `stderr`.
     * Returns an empty `Uint8Array` on EOF.
     *
     * @param maxBytes Maximum bytes to read (capped at 64 MiB).
     * @returns Promise resolving to the read chunk (`Uint8Array`).
     * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"` or `"VAEL_EXEC_IO"`) if `stderr` is not piped.
     * @since 0.0.0-alpha.2
     */
    readStderr(maxBytes?: number): Promise<Uint8Array>;

    /**
     * Send a termination/control signal to the subprocess (defaults to `"SIGTERM"`).
     * No-ops if the subprocess has already exited and been reaped.
     *
     * @param signal Signal to send (`"SIGTERM"`, `"SIGKILL"`, etc.).
     * @throws {Error} (`"VAEL_EXEC_UNSUPPORTED_SIGNAL"` or `"VAEL_EXEC_IO"`) if signaling fails.
     * @since 0.0.0-alpha.2
     */
    kill(signal?: Signal): void;

    /**
     * Non-blocking poll checking whether the subprocess has exited.
     *
     * @returns `CommandStatus` if exited, or `null` if still running.
     * @since 0.0.0-alpha.2
     */
    tryWait(): CommandStatus | null;

    /**
     * Wait for the subprocess to exit and reap it.
     *
     * @returns Promise resolving to `CommandStatus`.
     * @since 0.0.0-alpha.2
     */
    wait(): Promise<CommandStatus>;

    /**
     * Close any remaining `stdin`, concurrently drain `stdout` and `stderr`,
     * and wait for exit (enforcing optional `timeout` and `maxOutputBytes`).
     *
     * @param options Optional `{ maxOutputBytes, timeout }` bounds.
     * @returns Promise resolving to captured `Output`.
     * @since 0.0.0-alpha.2
     */
    waitWithOutput(options?: {
      /** Maximum bytes captured per output stream. @since 0.0.0-alpha.2 */
      maxOutputBytes?: number;
      /** Finite timeout in milliseconds. @since 0.0.0-alpha.2 */
      timeout?: number;
      /** Alias for `timeout` in milliseconds. @since 0.0.0-alpha.2 */
      timeoutMs?: number;
    }): Promise<Output>;

    /**
     * Alias for `waitWithOutput()`.
     *
     * @returns Promise resolving to captured `Output`.
     * @since 0.0.0-alpha.2
     */
    output(): Promise<Output>;

    /**
     * Dispose the child handle when leaving an `await using` block by sending
     * `"SIGTERM"` if still running and waiting for exit.
     *
     * @returns Promise resolving once the subprocess has been reaped.
     * @since 0.0.0-alpha.2
     */
    [Symbol.asyncDispose](): Promise<void>;
  }

  /**
   * Active OS advisory file lock guard returned by `lockFile` and `lockFileSync`.
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

    /**
     * Release the advisory file lock when exiting an `await using` scope.
     *
     * @returns Promise resolving after unlocking.
     * @since 0.0.0-alpha.2
     */
    [Symbol.asyncDispose](): Promise<void>;
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
   * import { cmd, execSync } from "vael:exec";
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
   * Execute a command asynchronously and capture `stdout` and `stderr`
   * (defaulting `stdin` to `"null"` and `stdout`/`stderr` to `"piped"`).
   *
   * Available in `vjs` (`vaeljs`) only; use `execSync` in `vjs-lite`.
   *
   * @param command Configured `Command` builder.
   * @returns Promise resolving to captured `Output`.
   * @throws {NotFoundOnPathError} if the executable cannot be found.
   * @throws {Error} with `code: ExecErrorCode` on invalid arguments, timeout, or output overflow.
   * @since 0.0.0-alpha.2
   */
  export function exec(command: Command): Promise<Output>;

  /**
   * Execute an `[program, ...args]` argv array asynchronously and capture
   * `stdout` and `stderr`.
   *
   * @param argv Non-empty `[program, ...args]` array (never shell-expanded).
   * @param options Optional working directory, environment, stdio, and limits.
   * @returns Promise resolving to captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function exec(argv: readonly string[], options?: CommandOptions): Promise<Output>;

  /**
   * Execute `program` asynchronously with optional `CommandOptions`.
   *
   * @param program Executable name or path.
   * @param options Optional arguments, working directory, environment, stdio, and limits.
   * @returns Promise resolving to captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function exec(program: string, options?: CommandOptions): Promise<Output>;

  /**
   * Execute `program` with `args` asynchronously and capture `stdout` and `stderr`.
   *
   * @param program Executable name or path.
   * @param args Literal argv arguments.
   * @param options Optional working directory, environment, stdio, and limits.
   * @returns Promise resolving to captured `Output`.
   * @since 0.0.0-alpha.2
   */
  export function exec(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): Promise<Output>;

  /**
   * Execute a command synchronously and capture `stdout` and `stderr`
   * (defaulting `stdin` to `"null"` and `stdout`/`stderr` to `"piped"`).
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
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
   * Execute a command asynchronously with inherited standard streams by default
   * and return exit status metadata.
   *
   * Available in `vjs` (`vaeljs`) only; use `runSync` in `vjs-lite`.
   *
   * @param command Configured `Command` builder.
   * @returns Promise resolving to `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function run(command: Command): Promise<CommandStatus>;

  /**
   * Execute an `[program, ...args]` argv array asynchronously with inherited
   * standard streams by default.
   *
   * @param argv Non-empty `[program, ...args]` array.
   * @param options Optional command configuration.
   * @returns Promise resolving to `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function run(argv: readonly string[], options?: CommandOptions): Promise<CommandStatus>;

  /**
   * Execute `program` asynchronously with inherited standard streams by default.
   *
   * @param program Executable name or path.
   * @param options Optional command configuration.
   * @returns Promise resolving to `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function run(program: string, options?: CommandOptions): Promise<CommandStatus>;

  /**
   * Execute `program` with `args` asynchronously with inherited standard
   * streams by default.
   *
   * @param program Executable name or path.
   * @param args Literal argv arguments.
   * @param options Optional command configuration.
   * @returns Promise resolving to `CommandStatus`.
   * @since 0.0.0-alpha.2
   */
  export function run(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): Promise<CommandStatus>;

  /**
   * Execute a command synchronously with inherited standard streams by default
   * and return exit status metadata.
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
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
   * Spawn a `Command` as a managed `ChildProcess` handle with piped streams by
   * default.
   *
   * Available in `vjs` (`vaeljs`) only.
   *
   * @param command Configured `Command` builder.
   * @returns Spawned `ChildProcess` handle.
   * @since 0.0.0-alpha.2
   */
  export function spawn(command: Command): ChildProcess;

  /**
   * Spawn an `[program, ...args]` argv array as a managed `ChildProcess` handle.
   *
   * @param argv Non-empty `[program, ...args]` array.
   * @param options Optional command configuration.
   * @returns Spawned `ChildProcess` handle.
   * @since 0.0.0-alpha.2
   */
  export function spawn(argv: readonly string[], options?: CommandOptions): ChildProcess;

  /**
   * Spawn `program` as a managed `ChildProcess` handle.
   *
   * @param program Executable name or path.
   * @param options Optional command configuration.
   * @returns Spawned `ChildProcess` handle.
   * @since 0.0.0-alpha.2
   */
  export function spawn(program: string, options?: CommandOptions): ChildProcess;

  /**
   * Spawn `program` with `args` as a managed `ChildProcess` handle.
   *
   * @param program Executable name or path.
   * @param args Literal argv arguments.
   * @param options Optional command configuration.
   * @returns Spawned `ChildProcess` handle.
   * @since 0.0.0-alpha.2
   */
  export function spawn(
    program: string,
    args: readonly string[],
    options?: CommandOptions,
  ): ChildProcess;

  /**
   * Asynchronously find the first executable matching `name` on `PATH`
   * (optionally prepended by `prependPath`).
   *
   * Available in `vjs` (`vaeljs`) only; use `whichSync` in `vjs-lite`.
   *
   * @param name Bare executable name or explicit path.
   * @param prependPath Optional `PATH`-formatted prefix searched before `PATH`.
   * @returns Promise resolving to the resolved executable path, or `null` if not found.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `name` is empty or contains NUL bytes.
   * @since 0.0.0-alpha.2
   */
  export function which(
    name: string,
    prependPath?: string | { readonly prependPath?: string },
  ): Promise<string | null>;

  /**
   * Synchronously find the first executable matching `name` on `PATH`
   * (optionally prepended by `prependPath`).
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
   *
   * @param name Bare executable name or explicit path.
   * @param prependPath Optional `PATH`-formatted prefix or options object `{ prependPath? }`.
   * @returns Resolved executable path, or `null` if not found.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `name` is empty or contains NUL bytes.
   * @since 0.0.0-alpha.2
   */
  export function whichSync(
    name: string,
    prependPath?: string | { readonly prependPath?: string },
  ): string | null;

  /**
   * Asynchronously find all executables matching `name` across `prependPath`
   * and `PATH` in search order.
   *
   * Available in `vjs` (`vaeljs`) only; use `whichAllSync` in `vjs-lite`.
   *
   * @param name Bare executable name or explicit path.
   * @param prependPath Optional `PATH`-formatted prefix or options object `{ prependPath? }`.
   * @returns Promise resolving to an array of matching executable paths.
   * @throws {Error} (`"VAEL_EXEC_INVALID_ARGUMENT"`) if `name` is empty or contains NUL bytes.
   * @since 0.0.0-alpha.2
   */
  export function whichAll(
    name: string,
    prependPath?: string | { readonly prependPath?: string },
  ): Promise<string[]>;

  /**
   * Synchronously find all executables matching `name` across `prependPath`
   * and `PATH` in search order.
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
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
   * Asynchronously acquire an OS advisory file lock on `path` (rejecting
   * symbolic links).
   *
   * Available in `vjs` (`vaeljs`) only; use `lockFileSync` in `vjs-lite`.
   *
   * @param path Target lock file path (must not be a symlink or end with `.`).
   * @param options Optional `{ mode, create, nonBlocking, timeout }` settings.
   * @returns Promise resolving to an active `FileLock` guard.
   * @throws {Error} with `code` `"VAEL_EXEC_LOCK_WOULD_BLOCK"`, `"VAEL_EXEC_LOCK_TIMEOUT"`, `"VAEL_EXEC_INVALID_ARGUMENT"`, or `"VAEL_EXEC_IO"`.
   * @since 0.0.0-alpha.2
   */
  export function lockFile(path: string, options?: LockOptions): Promise<FileLock>;

  /**
   * Synchronously acquire an OS advisory file lock on `path` (rejecting
   * symbolic links).
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
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

  /**
   * Executable registry for resolving and caching platform binary paths.
   *
   * @since 0.0.0-alpha.2
   */
  export class PathFinder {
    /**
     * Register or update executable lookup configuration under `name`.
     *
     * @param name Registry key name.
     * @param options Validated `PathFinderOptions` configuration.
     * @returns `this` for chaining.
     * @throws {TypeError} if `name` or `options` is invalid.
     * @since 0.0.0-alpha.2
     */
    set(name: string, options: PathFinderOptions): this;

    /**
     * Retrieve a shallow clone of the entry registered under exact key `name`.
     *
     * @param name Registry key name.
     * @returns Cloned `PathFinderOptions` or `undefined`.
     * @since 0.0.0-alpha.2
     */
    get(name: string): PathFinderOptions | undefined;

    /**
     * Return `true` if exact key `name` is registered.
     *
     * @param name Registry key name.
     * @since 0.0.0-alpha.2
     */
    has(name: string): boolean;

    /**
     * Delete exact key `name` from the registry.
     *
     * @param name Registry key name.
     * @returns `true` if an entry was removed.
     * @since 0.0.0-alpha.2
     */
    delete(name: string): boolean;

    /**
     * Remove all entries from the registry.
     *
     * @since 0.0.0-alpha.2
     */
    clear(): void;

    /**
     * Find a registered entry by exact key, `options.name`, `options.cached`,
     * or ASCII case-insensitive match.
     *
     * @param name Executable key, name, or cached path.
     * @returns Cloned `PathFinderOptions` or `undefined`.
     * @since 0.0.0-alpha.2
     */
    find(name: string): PathFinderOptions | undefined;

    /**
     * Synchronously resolve `name` to an executable file path via env
     * overrides, cache, `PATH`, and platform fallback candidates.
     *
     * @param name Executable key or name.
     * @returns Resolved executable path, or `undefined` if not found.
     * @since 0.0.0-alpha.2
     */
    findExeSync(name: string): string | undefined;

    /**
     * Asynchronously resolve `name` to an executable file path via env
     * overrides, cache, `PATH`, and platform fallback candidates.
     *
     * Available in `vjs` (`vaeljs`) only; use `findExeSync` in `vjs-lite`.
     *
     * @param name Executable key or name.
     * @returns Promise resolving to the executable path, or `undefined`.
     * @since 0.0.0-alpha.2
     */
    findExe(name: string): Promise<string | undefined>;
  }

  /**
   * Default singleton `PathFinder` registry consulted automatically when
   * executing a `Command` whose program matches a registered entry.
   *
   * @since 0.0.0-alpha.2
   */
  export const pathFinder: PathFinder;

  /**
   * Register a global command execution logging / audit handler.
   *
   * @param handler Callback invoked right before any command spawns.
   * @returns Disposable `CommandLogSubscription` guard.
   * @throws {TypeError} if `handler` is not a function.
   * @example
   * ```ts
   * import { addLogger, execSync } from "vael:exec";
   *
   * using _sub = addLogger((file, args) => console.log(file, args));
   * execSync("echo", ["hello"]);
   * ```
   * @since 0.0.0-alpha.2
   */
  export function addLogger(handler: CommandLogHandler): CommandLogSubscription;

  /**
   * Unregister all global command execution log handlers and mark existing
   * subscriptions inactive.
   *
   * @since 0.0.0-alpha.2
   */
  export function clearLoggers(): void;
}
