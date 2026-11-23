---
title: cron
description: UTC five-field cron parsing and in-process scheduling, with explicit overlap, missed-tick, and disposal rules.
---

`cron` is available in the **V8 runtime only**. It is not available in NeoJS Lite. Import it as
`cron` or `neo:cron`; the registry form is `Neo.getBuiltInModule("cron")`.

The module exports `cron`, `next`, and `CronError`. It is a UTC five-field parser and an in-process
scheduler. It is **not** Deno's `Deno.cron`, Bun's `Bun.cron`, `node-cron`, or a compatibility layer
for any of them.

## Example

This is the repository's `examples/cron.js`:

```js
import { cron, next } from "neo:cron";

// `next` is a pure UTC query: it creates no job and keeps nothing alive.
const upcoming = next("*/5 * * * *");
console.log(`next five-minute boundary (UTC): ${upcoming.toISOString()}`);

// A job keeps the process alive until it is disposed. This example schedules
// one and immediately disposes it, so it exits; remove the dispose to let the
// callback run every five minutes.
const job = cron("*/5 * * * *", () => {
    console.log("tick");
});
job.dispose();
```

The module is available in every V8 runtime; there is no secure-context or TLS gate.

## `next(expression, after?)`

`next` is a **pure query**: it creates no job, schedules nothing, and keeps nothing alive.

- `after` is a `Date` or an epoch millisecond number and defaults to the host clock.
- The result is a `Date` for the first occurrence **strictly after** `after`.
- The result is `null` when a valid expression has no occurrence in the 100-year search window
  (for example `0 0 31 2 *`), or when the next occurrence would fall outside the JavaScript `Date`
  range of ±8.64e15 milliseconds.
- Occurrences are whole UTC minutes, so seconds and milliseconds are always zero.

```js
import { next } from "neo:cron";

console.log(next("*/5 * * * *", new Date("2024-01-01T00:00:00Z")).toISOString());
// 2024-01-01T00:05:00.000Z
```

An invalid expression throws `CronError`. An invalid `Date` or a non-finite number is a
`RangeError`; any other `after` type is a `TypeError`.

## `cron(expression, callback, options?)`

`cron` parses the expression, creates a job, and returns a frozen controller. The first tick is
strictly after the call.

- `callback` is called with no arguments. A returned Promise is awaited before the next tick is
  considered available.
- `options` is optional. The only accepted key is `onEvent`, which must be a function if present.
  Unknown option keys are rejected with `TypeError`.
- An invalid expression throws `CronError`. A valid expression with no future occurrence throws
  `CronError` with `code` `NEO_CRON_NO_OCCURRENCE`.

The controller has:

| Member | Behaviour |
| --- | --- |
| `dispose()` | Idempotent; cancels the pending wait and releases the job |
| `[Symbol.dispose]` | Equal to `dispose` when the engine provides the symbol; non-enumerable |

There is no `ref`, `unref`, `close`, `closed`, name, backoff, or retry option. The job is frozen.

## Events

Pass `onEvent(event)` to observe scheduling events. This example is
self-contained: `* * * * *` runs once per UTC minute, so it waits up to 60
seconds for the first tick, observes a real `error` event, disposes the job,
and exits.

```js
import { cron, next } from "neo:cron";

// The first tick is strictly after this call, at a whole UTC minute.
const firstTick = next("* * * * *");
console.log(`waiting up to 60 seconds for ${firstTick.toISOString()}`);

let job;
job = cron("* * * * *", () => {
    throw new Error("demonstration failure");
}, {
    onEvent(event) {
        if (event.type !== "error") return;
        console.log(`observed ${event.type}: ${event.error.message} (fatal=${event.fatal})`);
        job.dispose();
    },
});
```

The callback throws on purpose so the example produces an `error` event.
`fatal` is `false` for a callback failure. If the expression were invalid or had
no future occurrence, `cron` would throw before a job is returned.

The event shapes are:

| Event | Fields |
| --- | --- |
| `overlap` | `{ type: "overlap", scheduledTime: Date, wokeAt: Date }` |
| `missed` | `{ type: "missed", scheduledTime: Date, wokeAt: Date }` |
| `error` | `{ type: "error", error, fatal }` |

- One callback runs at a time. A tick that arrives while the previous callback is still running is
  skipped and reported as `overlap`.
- A wake-up so late that the following occurrence is already due is reported as `missed`; the stale
  occurrence is not run, and the schedule continues from the current time instead of replaying a
  backlog.
- A thrown value or rejected Promise from the callback is reported as `error` and does not stop the
  schedule. `fatal` is `false` for callback failures; it is `true` only when the scheduler itself
  fails.
- An exception thrown by `onEvent` is discarded so the schedule keeps running.
- Without an `onEvent` observer, `overlap` and `missed` events are dropped, and callback failures are
  re-thrown from a microtask so they are not silently lost.

## Grammar

The expression is `minute hour day-of-month month day-of-week`.

| Field | Values | Names |
| --- | --- | --- |
| Minute | `0`–`59` | None |
| Hour | `0`–`23` | None |
| Day of month | `1`–`31` | None |
| Month | `1`–`12` | `JAN`–`DEC`; three-letter and full English names, case-insensitive |
| Day of week | `0`–`7`, where `0` and `7` are Sunday | `SUN`–`SAT`; three-letter and full English names, case-insensitive |

Each field accepts `*`, a value, a `start-end` range, or comma-separated lists, with an optional
`/step`. A step on a bare value runs from that value to the field maximum. Ranges must not wrap;
steps must be at least one and no greater than the field maximum. The extended `L`, `W`, `#`, and
`?` syntax is rejected. The whole expression is limited to 512 bytes.

When **both** day fields are restricted, a day matches when **either** matches, the POSIX/Vixie
rule. `*/2` counts as restricted, so `0 0 */2 * MON` means every second day of the month **or** any
Monday.

## Time and scheduling

All times are UTC. There is no local-time interpretation, DST handling, time-zone option, seconds
field, or `@daily`-style nickname.

Scheduling follows the host wall clock, including manual and NTP adjustments:

- A wait re-reads the clock at least once a minute and resolves only after the clock reaches the
  occurrence. A backward adjustment postpones a tick instead of running it early.
- The scheduler checks the clock once more immediately before dispatch.
- A forward adjustment is noticed within that minute. If it passes the following occurrence, the
  normal `missed` rule discards the stale tick instead of replaying a backlog.
- These checks bound the effect of a clock change; they do not make scheduling monotonic or
  independent of the system clock.

## Disposal and event-loop ownership

A job owns a pending host operation that sleeps until its next occurrence. That pending operation is
event-loop work:

- An **undisposed job keeps the isolate alive** until it is disposed or the `Runtime` is dropped.
- `dispose()` cancels the pending wait and is idempotent. It does **not** interrupt a callback that
  is already running.
- Dropping the runtime abandons every job.
- A synchronous callback blocks the isolate and therefore delays every timer.
- A callback that only awaits an ordinary Promise does not keep the process alive by itself.
- In a `neojs test` file, dispose every job. An undisposed job is pending event-loop work and makes
  the settle check fail.

The scheduler uses the runtime's Tokio timer, not a JavaScript fake-timer surface. The deterministic
test clock is compiled only with the off-by-default `test-support` feature and is absent from a
normal build.

## Limits

| Limit | Value |
| --- | --- |
| Expression size | 512 bytes |
| Live jobs per runtime | 1,024 |
| Occurrence search window | 100 years |
| Representable result | JavaScript `Date` range, ±8.64e15 ms |

A job whose next occurrence is the last representable minute dispatches that occurrence once and
then stops and releases its host resource. If the preceding callback is still in flight, the normal
overlap rule skips that final occurrence and reports `overlap`; the job still stops and releases.

## Errors

`CronError` extends `Error`, has `name = "CronError"`, and carries a stable own `code`. Match the
code, not the human-readable message.

| Code | Meaning |
| --- | --- |
| `NEO_CRON_EXPRESSION` | Not five fields, or a field is malformed or out of range |
| `NEO_CRON_LIMIT` | The expression or the number of live jobs exceeds a documented bound |
| `NEO_CRON_NO_OCCURRENCE` | A valid expression has no future occurrence in the search or `Date` range |
| `NEO_CRON_ARGUMENT` | A millisecond argument is not finite or in the `Date` range |

Bad JavaScript argument types throw `TypeError`; an invalid `Date` or non-finite epoch number throws
`RangeError` before a host operation runs.

## Scope

This module calls six host operations: parse and validate an expression, store a schedule, read the
host clock, compute the following occurrence, sleep on the Tokio timer, and cancel a job. It reads no
file, environment variable, network, or process state, and adds no capability beyond keeping the
event loop alive until the job is disposed. It is not a sandbox or a permission boundary.

There is no local-time, DST, time-zone, backoff, retry, or fake-timer surface. The scheduler is not
monotonic and does not promise exact dispatch at a wall-clock instant.
