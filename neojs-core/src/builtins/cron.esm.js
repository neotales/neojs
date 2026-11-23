// The `cron` built-in module as seen by `import ... from "cron"` or
// `import ... from "neo:cron"`.
//
// The module object lives in `neo.js`, which the runtime evaluates once per
// realm. This file only re-exports it, so every named export is the same value
// as the matching property of `Neo.getBuiltInModule("cron")`. Nothing here
// reaches the private module registry: it reads the public module object and
// binds its properties.
//
// `cron(expression, callback, options?)` returns a frozen controller.
// `dispose()` is idempotent, and `[Symbol.dispose]` is present when the engine
// provides the symbol. `next(expression, after?)` returns a `Date` or `null`.
// `CronError` is the class used for expression, limit, no-occurrence, and
// argument failures; it has an own `code`. Times are UTC and the schedule is a
// standard five-field expression. This is not Deno's `Deno.cron`, Bun's
// `Bun.cron`, `node-cron`, or a compatibility layer for any of them.
//
// The exported list must match the properties of the `cron` object in
// `neo.js`. `neojs-core/tests/cron.rs` asserts that the two lists are identical
// and that each export is identity-equal to the registry property.
const builtin = Neo.getBuiltInModule("cron");

export const cron = builtin.cron;
export const next = builtin.next;
export const CronError = builtin.CronError;
