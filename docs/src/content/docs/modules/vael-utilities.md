---
title: Utility Modules (UUID, Console, Cron, Stdio)
description: RFC 9562 UUIDs (vael:uuid), vael:console, V8-only UTC cron scheduling (vael:cron), and bounded stdio streams (vael:stdio).
---

This page covers VaelJS's general-purpose runtime utilities: `vael:uuid` (and subpath modules), `vael:console`, `vael:cron`, and `vael:stdio`.

## `vael:uuid`, `vael:uuid/v4` & `vael:uuid/v7`

:::note[Availability]
`vael:uuid`, `vael:uuid/v4`, and `vael:uuid/v7` are available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

All three modules generate canonical lowercase 36-character RFC 9562 UUID strings backed by the OS CSPRNG (with no insecure fallback). Supplying any argument throws `TypeError`.

| Specifier | Exports | Notes |
| --- | --- | --- |
| `vael:uuid` | `v4(): string`, `v7(): string` | `v4` is random; `v7` embeds current Unix ms in high 48 bits (not strictly monotonic within 1 ms). |
| `vael:uuid/v4` | `v4()`, `default` (`v4`) | Identical function object to `v4` from `"vael:uuid"`. |
| `vael:uuid/v7` | `v7()`, `default` (`v7`) | Identical function object to `v7` from `"vael:uuid"`. |

**`UuidErrorCode`**: `"VAEL_UUID_RANDOM"` (OS CSPRNG failure), `"VAEL_UUID_CLOCK"` (`v7` timestamp before Unix epoch or `>= 2^48` ms), `"VAEL_UUID_INTERNAL"`.

## `vael:console`

:::note[Availability]
`vael:console` is available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

`vael:console` exports the five methods of `globalThis.console` (`log`, `debug`, `info` to stdout; `warn`, `error` to stderr) as named exports, plus the live `console` (`VaelConsole`) object as `default`.

## `vael:cron` (`vjs` V8 Only)

:::note[Availability]
`vael:cron` is available in **`vjs` (V8) only**. It is **not** available in `vjs-lite`.
:::

`vael:cron` schedules UTC five-field cron jobs (`minute hour day-of-month month day-of-week`, max `512` bytes; extended `L`/`W`/`#`/`?` tokens are not supported).

| Export | Signature | Description |
| --- | --- | --- |
| `cron` | `(expression: string, callback: () => void \| PromiseLike<unknown>, options?: CronOptions) => CronJob` | Schedule a UTC job. Max `1024` active jobs concurrently. |
| `next` | `(expression: string, after?: Date \| number) => Date \| null` | Compute the first UTC occurrence strictly after `after` (within `366 * 100` days / `Date` range), or `null`. |
| `CronError` | `new CronError(code: CronErrorCode, message: string)` | Error with `name = "CronError"` and `code`. |

- **Execution & lifecycle**: One callback runs at a time; returned Promises are awaited before the next tick is eligible. Undisposed `CronJob` handles keep the event loop alive — call `job.dispose()` (or use `[Symbol.dispose]` when present) to cancel.
- **Observer (`CronOptions.onEvent`)**: Receives frozen events `{ type: "overlap", scheduledTime, wokeAt }`, `{ type: "missed", scheduledTime, wokeAt }`, or `{ type: "error", error, fatal }`. Errors thrown by `onEvent` itself are discarded.
- **`CronErrorCode`**: `"VAEL_CRON_EXPRESSION"` | `"VAEL_CRON_LIMIT"` | `"VAEL_CRON_NO_OCCURRENCE"` | `"VAEL_CRON_ARGUMENT"`.

## `vael:stdio` (`vjs` V8 Only)

:::note[Availability]
`vael:stdio` is available in **`vjs` (V8) only**. It is **not** available in `vjs-lite`.
:::

`vael:stdio` provides bounded synchronous in-memory streams and shared writers targeting process `stdout` and `stderr`.

| Export | Type / Signature | Description |
| --- | --- | --- |
| `MAX_BYTES` | `8388608` | Maximum capacity (`8 MiB`) of a memory-backed stream or `writeSync` chunk. |
| `Readable` | `new Readable(bytes: Uint8Array)` | Copies `bytes` (max `MAX_BYTES`). Exposes `remaining`, `closed`, `readSync(target): number`, `close()`. |
| `Writable` | `new Writable()` | Empty memory writer (max `MAX_BYTES`). Exposes `kind`, `length`, `closed`, `ended`, `writeSync(chunk): number`, `end()`, `toUint8Array()`, `close()`. |
| `stdout`, `stderr` | `Writable` | Shared host writers (`kind = "stdout" \| "stderr"`). `toUint8Array()` throws `VAEL_STDIO_NOT_MEMORY`; `close()` retires the JS handle without closing the OS fd. |

**`StdioErrorCode`** (`name = "StdioError"`): `"VAEL_STDIO_DETACHED"` | `"VAEL_STDIO_LIMIT"` | `"VAEL_STDIO_ARGUMENT"` | `"VAEL_STDIO_WRITE"` | `"VAEL_STDIO_CLOSED"` | `"VAEL_STDIO_ENDED"` | `"VAEL_STDIO_NOT_MEMORY"`.

## Example

```ts
import { cron, next } from "vael:cron";
import { Writable, stdout } from "vael:stdio";
import { v4 } from "vael:uuid/v4";
import { v7 } from "vael:uuid/v7";

console.log("v4:", v4(), "v7:", v7());
console.log("next midnight UTC:", next("0 0 * * *")?.toISOString());

const buf = new Writable();
buf.writeSync("buffered line\n");
buf.end();
stdout.writeSync(buf.toUint8Array());
buf.close();

const job = cron("* * * * *", () => {
  console.log("tick", new Date().toISOString());
});
job.dispose();
```
