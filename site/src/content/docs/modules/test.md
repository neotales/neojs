---
title: test
description: The registration-based test runner behind neojs test, including timeouts, pending tests, and failure conditions.
---

`test` is available in every V8 runtime and is **not** available in NeoJS Lite. Import it as `test`
or `neo:test`; the registry form is `Neo.getBuiltInModule("test")`.

The default export is the `test` function. The named exports are `test`, `run`, `MAX_TESTS`, and
`TestFailureError`.

This is not Node's `node:test`. `node:test` is rejected, and there is no Node-module opt-in.

## Example

`math.test.js`:

```js
import test from "test";
import assert from "assert";

test("adds", () => {
    assert.equals(1 + 1, 2);
});

test("adds asynchronously", async () => {
    await Promise.resolve();
    assert.equals(2 + 2, 4);
});
```

```sh
neojs test "tests/**/*.test.js" "tests/smoke.test.js"
```

```text
test /repo/tests/math.test.js:
ok 1 - adds
ok 2 - adds asynchronously
# 2 tests, 2 passed, 0 failed
# 1 selected file(s): 2 test(s): 2 passed, 0 failed, 0 pending, 0 unhandled, 0 unsettled; 0 file(s) failed
```

## API

### `test(name, fn[, options])`

Registers a test. `name` must be a non-empty string. `fn` may be synchronous or asynchronous; a
returned thenable is awaited. A generator function is rejected at registration, and a returned
generator object is a failure because the body did not run.

`options.timeout` sets the per-test timeout in milliseconds. The default is 30,000:

```js
test("slow", async () => {
    // ...
}, { timeout: 60_000 });
```

### `test(name)`

Registers a **pending** test. A pending test fails the run.

### `run([options])`

Runs every queued test in registration order and returns cumulative counts. `run` is called by the
`neojs test` host after the entry module settles, so a test registered after a top-level `await` is
included.

- `options.throwOnFailure` defaults to `true`. It throws `TestFailureError` whenever the cumulative
  report has any failure, including one retained from an earlier call.
- `options.settle` defaults to `false` and is for the host's final bounded event-loop check. A user
  call inside a top-level `await` must leave it `false`, because it would see its own pending module
  evaluation.

### `MAX_TESTS`

`10000`. Registering more than this in one runtime throws `NEO_TEST_LIMIT`.

### `TestFailureError`

The error thrown by `run` when a report has failures. It has `name = "TestFailureError"` and a
stable own `code` of `NEO_TEST_FAILURES`, plus the `failures` report.

## Failure conditions

These are failures, so none can look like a pass:

- A rejected Promise or rejected thenable.
- A returned generator object.
- A pending test.
- An unhandled rejection.
- A test that does not settle within its timeout.
- Event-loop work that remains after the bounded settle window.
- A file that registers no tests.

A failed file does not stop later files. Each file gets its own isolate, so globals, module state,
and the queue do not leak. A file that fails is reported, and the command exits 1.

## Promise handling

Native Promises are observed through their real internal state rather than `then`, `constructor`, or
`Symbol.species`. A non-native thenable is read once, called once, and its first settlement wins.
Nested promises and thenables are assimilated recursively. Cycles fail with `NEO_TEST_CYCLE`, and a
chain longer than 1,000 steps fails with `NEO_TEST_CHAIN`. The per-test timeout covers the whole
chain.

`NEO_TEST_OBSERVE` means the V8 observer could not inspect a native Promise. `NEO_TEST_NO_TIMER`
means timer support was unavailable and the runner failed closed rather than hang. A synchronous
loop or a thenable that blocks or starves the isolate is still not interruptible, and a timeout
fails the test without cancelling work that is already running.

After the queue, the host waits up to about 100 ms of event-loop turns for detached timers, async
ops, dynamic imports, and rejection events. If `deno_core` still reports pending work, the report is
marked unsettled and fails; an unrefed task that the event loop does not report is outside this
guarantee. Work scheduled at module top level is part of module evaluation, so a refed top-level
timer can keep the entry waiting before the test runner starts; keep detached work inside test
callbacks or do not schedule it.

## Registration errors

| Code | Meaning |
| --- | --- |
| `NEO_TEST_DURING_RUN` | `test` called while `run` is active |
| `NEO_TEST_LIMIT` | More than `MAX_TESTS` tests registered in one runtime |
| `NEO_TEST_UNSUPPORTED` | A generator function or returned generator object |
| `NEO_TEST_ALREADY_RUNNING` | `run` called while already running |

A bad name, callback, or options object is a `TypeError`.

## Limits

There is no default discovery pattern, watch mode, parallelism, filtering, retries, subtests, hooks,
mocks, or snapshots, and no full Node compatibility claim. Test files run with the process's full
filesystem authority; this is not a sandbox. A synchronous callback that blocks the isolate forever
still hangs the process.
