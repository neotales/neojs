---
title: vael:test & vael:assert
description: Registration-based test runner and structural/async assertions executed via vjs test and vjs-lite test.
---

`vael:test` and `vael:assert` form VaelJS's built-in test framework and assertion library.

:::note[Availability]
Both `vael:test` and `vael:assert` are available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**. However, `test(..., { timeout })` is **V8-only** — passing `timeout` in `vjs-lite` throws `VAEL_TEST_NO_TIMER` because QuickJS has no host timers.
:::

## `vael:test`

`vael:test` uses flat registration (`test()` + `run()`); it is **not** `node:test` and does not export `describe`, `skip`, or `only`. Registering `test(name)` without a callback marks the test as **pending** (which fails the run). Generator callbacks are rejected with `VAEL_TEST_UNSUPPORTED`.

### Exports

| Export | Type / Signature | Description |
| --- | --- | --- |
| `test` | `(name: string, fn?: () => unknown, options?: TestOptions) => void` | Register a sync/async test case (or pending test if `fn` omitted). Also the `default` export. |
| `run` | `(options?: RunOptions) => Promise<TestReport>` | Execute queued tests in registration order and return `TestReport`. |
| `MAX_TESTS` | `10000` | Maximum tests registered in one runtime. |
| `TestFailureError` | `class TestFailureError extends Error` | Thrown by `run()` on failures (`name = "TestFailureError"`, `code = "VAEL_TEST_FAILURES"`, `failures: string[]`). |

### Options & Report Shapes

- **`TestOptions`**: `{ timeout?: number }` (`1..=2_147_483_647` ms, default `30_000` in `vjs`; throws `VAEL_TEST_NO_TIMER` in `vjs-lite`).
- **`RunOptions`**: `{ throwOnFailure?: boolean; settle?: boolean }` (`throwOnFailure` defaults to `true`; `settle` defaults to `false`).
- **`TestReport`**: `{ total: number, passed: number, failed: number, pending: number, unhandled: number, unsettled: number, failures: string[] }`.

### Error Codes (`TestErrorCode`)

`VAEL_TEST_FAILURES` | `VAEL_TEST_DURING_RUN` | `VAEL_TEST_ALREADY_RUNNING` | `VAEL_TEST_LIMIT` | `VAEL_TEST_UNSUPPORTED` | `VAEL_TEST_NO_TIMER` | `VAEL_TEST_PENDING` | `VAEL_TEST_CYCLE` | `VAEL_TEST_CHAIN` | `VAEL_TEST_OBSERVE`

## `vael:assert`

`assert` is exported both as a named export, as the `default` export, and as `ok`, with all assertion functions attached as properties on the callable function object. Failures throw `AssertionError` (`name = "AssertionError"`, `code = "VAEL_ASSERT"`, `operator`, `actual?`, `expected?`).

| Export | Signature | Behavior |
| --- | --- | --- |
| `assert` / `ok` | `(value: unknown, message?: string) => asserts value` | Assert truthiness. |
| `equals` | `(actual, expected, message?) => void` | Strict structural equality (prototypes match, `NaN === NaN`, `-0 !== 0`, order-insensitive `Map`/`Set`, cycle-safe). |
| `notEquals` | `(actual, expected, message?) => void` | Assert structural inequality. |
| `strictEquals` | `(actual, expected, message?) => void` | Identity equality via `Object.is(actual, expected)`. |
| `notStrictEquals` | `(actual, expected, message?) => void` | Inequality under `Object.is(actual, expected)`. |
| `fail` | `(message?: string) => never` | Unconditionally throw `AssertionError`. |
| `throws` | `(callback, expected?, message?) => unknown` | Sync only: assert `callback` throws (class or `RegExp`) and return thrown value. Rejects Promise-returning callbacks with `TypeError`. |
| `doesNotThrow` | `<T>(callback: () => T, message?) => T` | Sync only: assert `callback` returns without throwing and return its result. |
| `rejects` | `(callback, expected?, message?) => Promise<unknown>` | Assert `callback` rejects (or throws) matching optional class/`RegExp`; resolves with reason. |
| `doesNotReject` | `<T>(callback: () => PromiseLike<T>, message?) => Promise<T>` | Assert `callback` fulfills; resolves with value. |
| `AssertionError` | `new AssertionError(message?, options?)` | Error class (`options?: { operator?, actual?, expected? }`). |

## Running Tests via the CLI

Run test suites directly with `vjs test` or `vjs-lite test`:

```sh
vjs test
vjs test tests/**/*.test.ts
vjs-lite test tests/unit.test.ts
```

## Example

```ts
import assert, { equals, rejects, throws } from "vael:assert";
import { test } from "vael:test";

test("structural equality and sync throws", () => {
  assert(1 + 1 === 2);
  equals(new Set([1, 2]), new Set([2, 1]));
  throws(() => {
    throw new RangeError("bad index");
  }, RangeError);
});

test("async rejection", async () => {
  await rejects(async () => {
    throw new Error("boom");
  }, /boom/);
});
```
