/**
 * UTC five-field cron scheduler and next-occurrence calculator.
 *
 * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
 * Evaluates standard five-field UTC expressions (`minute hour dom month dow`)
 * with overlap and missed-tick protection.
 *
 * @example
 * ```ts
 * import { cron, next } from "vael:cron";
 *
 * const upcoming = next("0 9 * * 1-5", new Date("2026-01-01T00:00:00Z"));
 * console.log(upcoming?.toISOString());
 *
 * const job = cron("* * * * *", () => {
 *   console.log("tick");
 * });
 * job.dispose();
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:cron" {
  /**
   * Error codes reported on `CronError.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type CronErrorCode =
    | "VAEL_CRON_EXPRESSION"
    | "VAEL_CRON_LIMIT"
    | "VAEL_CRON_NO_OCCURRENCE"
    | "VAEL_CRON_ARGUMENT";

  /**
   * Error thrown for invalid cron expressions, job limits, missing future
   * occurrences, or invalid cron arguments.
   *
   * @since 0.0.0-alpha.0
   */
  export class CronError extends Error {
    /**
     * Error name (`"CronError"`).
     *
     * @since 0.0.0-alpha.0
     */
    override name: "CronError";

    /**
     * Machine-readable cron error code.
     *
     * @since 0.0.0-alpha.0
     */
    code: CronErrorCode;

    /**
     * Construct a `CronError`.
     *
     * @param code Machine-readable `CronErrorCode`.
     * @param message Human-readable error message.
     * @since 0.0.0-alpha.0
     */
    constructor(code: CronErrorCode, message: string);
  }

  /**
   * Scheduler lifecycle event passed to `options.onEvent`.
   *
   * @since 0.0.0-alpha.0
   */
  export type CronEvent =
    | {
        /** Tick arrived while the previous callback was still running and was skipped. @since 0.0.0-alpha.0 */
        readonly type: "overlap";
        /** Scheduled UTC occurrence time. @since 0.0.0-alpha.0 */
        readonly scheduledTime: Date;
        /** Actual UTC wake-up time. @since 0.0.0-alpha.0 */
        readonly wokeAt: Date;
      }
    | {
        /** Wake-up was late enough that the following occurrence was already due; stale tick was skipped. @since 0.0.0-alpha.0 */
        readonly type: "missed";
        /** Scheduled UTC occurrence time that was missed. @since 0.0.0-alpha.0 */
        readonly scheduledTime: Date;
        /** Actual UTC wake-up time. @since 0.0.0-alpha.0 */
        readonly wokeAt: Date;
      }
    | {
        /** Callback threw synchronously, returned a rejected Promise, or the scheduler encountered a wait error. @since 0.0.0-alpha.0 */
        readonly type: "error";
        /** Thrown or rejected value. @since 0.0.0-alpha.0 */
        readonly error: unknown;
        /** `true` if a fatal scheduler error terminated the job; `false` for a callback error. @since 0.0.0-alpha.0 */
        readonly fatal: boolean;
      };

  /**
   * Options accepted by `cron(expression, callback, options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface CronOptions {
    /**
     * Observer invoked with frozen `overlap`, `missed`, or `error` events.
     * Errors thrown by `onEvent` itself are discarded so the schedule stays alive.
     *
     * @since 0.0.0-alpha.0
     */
    onEvent?: (event: CronEvent) => void;
  }

  /**
   * Frozen handle controlling a scheduled UTC cron job.
   *
   * Undisposed jobs keep the event loop alive. `[Symbol.dispose]` is also
   * installed as an alias for `dispose` when `Symbol.dispose` exists in the engine.
   *
   * @since 0.0.0-alpha.0
   */
  export interface CronJob {
    /**
     * Cancel any pending timer wait and release the host job resource.
     * Idempotent; does not interrupt a callback that is already running.
     *
     * @since 0.0.0-alpha.0
     */
    dispose(): void;
  }

  /**
   * Schedule `callback` on a UTC five-field cron expression
   * (`minute hour day-of-month month day-of-week`, max 512 bytes).
   *
   * At most 1024 active jobs may exist concurrently. One callback executes at a
   * time; returned Promises are awaited before the next tick is eligible.
   *
   * @param expression Five-field UTC cron expression (no extended `L`/`W`/`#`/`?`).
   * @param callback Invoked with no arguments on each eligible tick.
   * @param options Optional `{ onEvent }` observer.
   * @returns Frozen `CronJob` handle whose `dispose()` stops future ticks.
   * @throws {TypeError} on invalid arguments or unknown options.
   * @throws {CronError} with `code` `"VAEL_CRON_EXPRESSION"`, `"VAEL_CRON_LIMIT"`,
   * `"VAEL_CRON_NO_OCCURRENCE"`, or `"VAEL_CRON_ARGUMENT"`.
   * @example
   * ```ts
   * import { cron } from "vael:cron";
   *
   * const job = cron("0 * * * *", async () => {
   *   console.log("hourly job");
   * }, {
   *   onEvent: (ev) => console.warn(ev.type),
   * });
   * job.dispose();
   * ```
   * @since 0.0.0-alpha.0
   */
  export function cron(
    expression: string,
    callback: () => void | PromiseLike<unknown>,
    options?: CronOptions,
  ): CronJob;

  /**
   * Compute the first UTC occurrence strictly after `after`, or `null` if none
   * exists within the 100-year (`366 * 100` days) search span or `Date` range.
   *
   * @param expression Five-field UTC cron expression (max 512 bytes).
   * @param after Reference `Date` or epoch milliseconds (defaults to current host UTC time).
   * @returns Next matching UTC `Date`, or `null` when no occurrence exists in range.
   * @throws {TypeError} when `expression` or `after` has an invalid type.
   * @throws {RangeError} when `after` is an invalid `Date` or non-finite number.
   * @throws {CronError} with `code` `"VAEL_CRON_EXPRESSION"` or `"VAEL_CRON_ARGUMENT"`.
   * @example
   * ```ts
   * import { next } from "vael:cron";
   *
   * const after = new Date("2026-01-01T00:00:00Z");
   * console.log(next("30 12 * * *", after)?.toISOString());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function next(expression: string, after?: Date | number): Date | null;
}
