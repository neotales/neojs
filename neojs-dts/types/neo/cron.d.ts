declare module "neo:cron" {
  /**
   * Error codes reported on `CronError.code`.
   */
  export type CronErrorCode =
    | "NEO_CRON_EXPRESSION"
    | "NEO_CRON_LIMIT"
    | "NEO_CRON_NO_OCCURRENCE"
    | "NEO_CRON_ARGUMENT";

  /**
   * Error thrown for invalid cron expressions, job limits, missing future
   * occurrences, or invalid cron arguments.
   */
  export class CronError extends Error {
    override name: "CronError";
    code: CronErrorCode;
    constructor(code: CronErrorCode, message: string);
  }

  /**
   * Scheduler lifecycle event passed to `options.onEvent`.
   */
  export type CronEvent =
    | {
        /** Tick arrived while the previous callback was still running and was skipped. */
        readonly type: "overlap";
        /** Scheduled UTC occurrence time. */
        readonly scheduledTime: Date;
        /** Actual UTC wake-up time. */
        readonly wokeAt: Date;
      }
    | {
        /** Wake-up was late enough that the following occurrence was already due; stale tick was skipped. */
        readonly type: "missed";
        /** Scheduled UTC occurrence time that was missed. */
        readonly scheduledTime: Date;
        /** Actual UTC wake-up time. */
        readonly wokeAt: Date;
      }
    | {
        /** Callback threw synchronously, returned a rejected Promise, or the scheduler encountered a wait error. */
        readonly type: "error";
        /** Thrown or rejected value. */
        readonly error: unknown;
        /** `true` if a fatal scheduler error terminated the job; `false` for a callback error. */
        readonly fatal: boolean;
      };

  /**
   * Options accepted by `cron(expression, callback, options)`.
   */
  export interface CronOptions {
    /**
     * Observer invoked with frozen `overlap`, `missed`, or `error` events.
     * Errors thrown by `onEvent` itself are discarded so the schedule stays alive.
     */
    onEvent?: (event: CronEvent) => void;
  }

  /**
   * Frozen handle controlling a scheduled UTC cron job.
   *
   * Undisposed jobs keep the event loop alive. `[Symbol.dispose]` is also
   * installed as an alias for `dispose` when `Symbol.dispose` exists in the engine.
   */
  export interface CronJob {
    /**
     * Cancel any pending timer wait and release the host job resource.
     * Idempotent; does not interrupt a callback that is already running.
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
   * @throws {TypeError} on invalid arguments or unknown options.
   * @throws {CronError} with `code` `"NEO_CRON_EXPRESSION"`, `"NEO_CRON_LIMIT"`,
   * `"NEO_CRON_NO_OCCURRENCE"`, or `"NEO_CRON_ARGUMENT"`.
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
   * @throws {TypeError} when `expression` or `after` has an invalid type.
   * @throws {RangeError} when `after` is an invalid `Date` or non-finite number.
   * @throws {CronError} with `code` `"NEO_CRON_EXPRESSION"` or `"NEO_CRON_ARGUMENT"`.
   */
  export function next(expression: string, after?: Date | number): Date | null;
}
