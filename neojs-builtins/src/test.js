(function createNeoTest(host) {
    "use strict";

    // The `test` built-in module.
    //
    // `test(name, fn)` queues a callback, and `run()` executes the queue in
    // registration order. The CLI calls `run()` after the entry module
    // settles, so tests registered after a top-level await are included. A
    // callback may return a Promise; it is awaited and a rejection is a
    // failure. A callback that returns a generator object is rejected rather
    // than silently skipped. A test registered without a callback is pending
    // and fails the run. An unhandled rejection that occurs while the module
    // or the runner is active is recorded and fails the run.
    //
    // This is not Node's `node:test`. There are no subtests, hooks, mocks,
    // snapshots, tags, filters, retries, or parallel execution, and `test()`
    // returns `undefined` rather than a Promise. Tests share one realm per
    // file; the CLI gives each file its own Runtime. The runner is not a
    // sandbox, and a callback that blocks the isolate forever cannot be
    // interrupted.
    //
    // The bookkeeping below avoids mutable prototypes for the queue and the
    // failure list, so replacing array methods after this shim runs cannot drop
    // a registration. Native Promises are observed through the host's native
    // Promise observation hook, and non-native thenables are assimilated
    // recursively with first-settlement, cycle, and depth guards. Replacing
    // Promise.resolve, Promise.prototype.then, constructor, or Symbol.species
    // does not turn a pending test into a pass.
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const NativeError = Error;
    const NativeString = String;
    const NativePromise = Promise;
    const nativePromiseResolve = NativePromise.resolve.bind(NativePromise);
    const NativeSymbol = Symbol;
    const isInteger = Number.isInteger;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const objectKeys = Object.keys;
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const arrayJoin = Function.prototype.call.bind(Array.prototype.join);
    const uncurry = Function.prototype.call.bind(Function.prototype.call);
    const functionHasInstance = Function.prototype[NativeSymbol.hasInstance];
    const testObjectToString = Object.prototype.toString;
    const toWellFormed =
        typeof String.prototype.toWellFormed === "function"
            ? String.prototype.toWellFormed
            : null;

    if (
        !host ||
        typeof host.isNativePromise !== "function" ||
        typeof host.observePromise !== "function" ||
        typeof host.print !== "function"
    ) {
        throw new makeTypeError(
            "neo:test host requires isNativePromise, observePromise, and print hooks",
        );
    }

    const coreIsPromise = host.isNativePromise;
    const observePromiseOp = host.observePromise;
    const testPrint = host.print;
    const testCreateTimer = host.createTimer;
    const testCancelTimer = host.cancelTimer;
    const hasTimers =
        typeof testCreateTimer === "function" && typeof testCancelTimer === "function";
    const testSetUnhandledRejectionHandler = host.setUnhandledRejectionHandler;
    const testEventLoopHasMoreWork = host.eventLoopHasMoreWork;
    const testFlushUnhandled = host.flushUnhandledRejections;

    const OBSERVE_PENDING = NativeSymbol("test.pending");
    const OBSERVE_FULFILLED = NativeSymbol("test.fulfilled");
    const OBSERVE_REJECTED = NativeSymbol("test.rejected");
    const OBSERVE_FAILED = NativeSymbol("test.inspection-failed");

    const TEST_MAX = 10000;
    const TEST_DEFAULT_TIMEOUT_MS = 30000;
    const TEST_MAX_TEXT = 2000;
    const TEST_SETTLE_ROUNDS = 100;
    const TEST_MAX_CHAIN = 1000;

    let testQueueLength = 0;
    const testQueue = [];
    let testCursor = 0;
    let testRegistered = 0;
    let testRunning = false;
    let testTotal = 0;
    let testPassed = 0;
    let testFailed = 0;
    let testPending = 0;
    let testUnhandled = 0;
    let testUnsettled = 0;
    let testUnsettledRecorded = false;
    const testFailures = [];
    let testHandlerInstalled = false;

    function defineField(object, key, value) {
        defineProperty(object, key, {
            __proto__: null,
            value,
            enumerable: true,
            configurable: true,
            writable: true,
        });
    }

    function append(array, value) {
        defineField(array, array.length, value);
    }

    function contains(array, value) {
        for (let index = 0; index < array.length; index += 1) {
            if (array[index] === value) return true;
        }
        return false;
    }

    function hasBrand(value, constructor) {
        try {
            return uncurry(functionHasInstance, constructor, value);
        } catch {
            return false;
        }
    }

    function observePromise(promise) {
        const observation = {
            status: OBSERVE_PENDING,
            value: undefined,
            error: undefined,
        };
        observation.promise = new NativePromise(function (resolve) {
            try {
                observePromiseOp(
                    promise,
                    function (value) {
                        observation.status = OBSERVE_FULFILLED;
                        observation.value = value;
                        resolve(undefined);
                    },
                    function (reason) {
                        observation.status = OBSERVE_REJECTED;
                        observation.value = reason;
                        resolve(undefined);
                    },
                );
            } catch (error) {
                observation.status = OBSERVE_FAILED;
                observation.error = error;
                resolve(undefined);
            }
        });
        return observation;
    }

    function testWrite(text, isError) {
        try {
            testPrint(testWellFormed(text), isError === true);
        } catch {
            // A closed descriptor must not hide the counts.
        }
    }

    function testPrintLine(text) {
        testWrite(`${text}\n`, false);
    }

    function testWellFormed(text) {
        try {
            if (typeof toWellFormed === "function") return uncurry(toWellFormed, text);
        } catch {
            // Fall through to the raw text.
        }
        return text;
    }

    function testText(value) {
        try {
            return NativeString(value);
        } catch {
            return "unprintable text";
        }
    }

    function testOneLine(text, limit) {
        const value = testText(text);
        let out = "";
        let count = 0;
        for (let index = 0; index < value.length; index += 1) {
            if (count >= limit) return `${out}...`;
            const character = value[index];
            if (character === "\n" || character === "\r") {
                if (out.length === 0 || out[out.length - 1] !== " ") out += " ";
                count += 1;
                continue;
            }
            out += character;
            count += 1;
        }
        return out;
    }

    function testLabel(name) {
        return testOneLine(name, 200);
    }

    function testError(code, message, kind) {
        const error =
            kind === "TypeError" ? new makeTypeError(message) : new NativeError(message);
        defineField(error, "code", code);
        return error;
    }

    function testDescribeError(error) {
        try {
            if (hasBrand(error, NativeError)) {
                let name = "Error";
                let code = "";
                let message = "";
                try {
                    if (typeof error.name === "string" && error.name) name = error.name;
                } catch {
                    // Keep the fallback name.
                }
                try {
                    if (typeof error.code === "string" && error.code) code = ` [${error.code}]`;
                } catch {
                    // Keep the fallback code.
                }
                try {
                    message = testOneLine(error.message, TEST_MAX_TEXT);
                } catch {
                    message = "<message unreadable>";
                }
                return `${name}${code}: ${message}`;
            }
            if (typeof error === "string") {
                return `thrown string: ${testOneLine(error, TEST_MAX_TEXT)}`;
            }
            return `thrown ${typeof error}: ${testOneLine(testText(error), TEST_MAX_TEXT)}`;
        } catch {
            return "thrown value";
        }
    }

    function testPrintIndented(text) {
        const value = testText(text);
        let start = 0;
        for (let index = 0; index <= value.length; index += 1) {
            if (index !== value.length && value[index] !== "\n") continue;
            testPrintLine(`  ${stringSlice(value, start, index)}`);
            start = index + 1;
            if (index - start > TEST_MAX_TEXT) break;
        }
    }

    function testPrintStack(error) {
        let stack = "";
        try {
            if (!hasBrand(error, NativeError) || typeof error.stack !== "string") return;
            stack = error.stack;
        } catch {
            return;
        }
        let start = 0;
        let printed = 0;
        for (let index = 0; index <= stack.length && printed < 20; index += 1) {
            if (index !== stack.length && stack[index] !== "\n") continue;
            const line = stringSlice(stack, start, index);
            if (stringSlice(line, 0, 5) === "    at " || stringSlice(line, 0, 3) === "  at ") {
                testPrintLine(`  ${line}`);
                printed += 1;
            }
            start = index + 1;
        }
    }

    function testFunctionKind(value) {
        try {
            const tag = uncurry(testObjectToString, value);
            if (tag === "[object GeneratorFunction]" || tag === "[object AsyncGeneratorFunction]") {
                return "generator";
            }
        } catch {
            // A proxy or a broken prototype is handled by the runtime check.
        }
        return "ordinary";
    }

    function testGeneratorObject(value) {
        if (value === null || (typeof value !== "object" && typeof value !== "function")) {
            return false;
        }
        try {
            const tag = uncurry(testObjectToString, value);
            return tag === "[object Generator]" || tag === "[object AsyncGenerator]";
        } catch {
            return false;
        }
    }

    function testThen(value) {
        if (value === null || (typeof value !== "object" && typeof value !== "function")) {
            return null;
        }
        const then = value.then;
        return typeof then === "function" ? then : null;
    }

    function testSettledWrapper(status, value) {
        const wrapper = { __proto__: null };
        defineField(wrapper, "status", status);
        defineField(wrapper, "value", value);
        return wrapper;
    }

    /**
     * Call a non-native thenable's `then` once and capture its first settlement.
     *
     * The wrapper is a non-thenable object, so native Promise resolution never
     * assimilates the payload. A nested native Promise is left for
     * `testResolveValue`, which observes it through the host instead of mutable
     * `Promise.prototype.then`. A second resolve/reject or a throw after the
     * first settlement is ignored.
     */
    function testCallThenable(value, then) {
        return new NativePromise(function (resolve) {
            let settled = false;
            const settle = function (status, payload) {
                if (settled) return;
                settled = true;
                resolve(testSettledWrapper(status, payload));
            };
            try {
                uncurry(
                    then,
                    value,
                    function (fulfilled) {
                        settle("fulfilled", fulfilled);
                    },
                    function (rejected) {
                        settle("rejected", rejected);
                    },
                );
            } catch (error) {
                settle("rejected", error);
            }
        });
    }

    function testCycleError(name) {
        return testError(
            "NEO_TEST_CYCLE",
            `test "${name}" produced a cyclic thenable chain`,
            "Error",
        );
    }

    function testChainError(name) {
        return testError(
            "NEO_TEST_CHAIN",
            `test "${name}" exceeded the ${TEST_MAX_CHAIN}-step thenable chain limit`,
            "Error",
        );
    }

    function testObserveError(name, error) {
        let detail = "";
        try {
            detail = `: ${testDescribeError(error)}`;
        } catch {
            // The class name is still useful without a detail.
        }
        return testError(
            "NEO_TEST_OBSERVE",
            `could not inspect the Promise returned by "${name}"${detail}`,
            "Error",
        );
    }

    /**
     * Recursively reduce a test result to a non-thenable wrapper.
     *
     * A genuine native Promise is observed through the host's native Promise
     * observation hook, which attaches settlement handlers without reading
     * `then`, `constructor`, or `Symbol.species`. A non-native thenable has
     * its `then` read once, is called once, and its first settlement is
     * captured without letting native resolution assimilate the payload.
     * Cycles and chains longer than `TEST_MAX_CHAIN` fail rather than
     * starving the timeout.
     */
    async function testResolveValue(value, seen, depth, name) {
        if (depth > TEST_MAX_CHAIN) throw testChainError(name);
        // Check identity before reading `then` again. A stateful getter can
        // return a function on the first read and undefined or throw on the
        // second; reading it first would turn a cycle into a pass or mask the
        // cycle with the getter's later error. Plain objects are never added to
        // `seen`, so an ordinary value that is encountered again is not a cycle.
        if (contains(seen, value)) throw testCycleError(name);
        if (coreIsPromise(value)) {
            append(seen, value);
            const observation = observePromise(value);
            await observation.promise;
            if (observation.status === OBSERVE_REJECTED) throw observation.value;
            if (observation.status !== OBSERVE_FULFILLED) {
                throw testObserveError(name, observation.error);
            }
            return await testResolveValue(observation.value, seen, depth + 1, name);
        }
        const then = testThen(value);
        if (then === null) return testSettledWrapper("fulfilled", value);
        append(seen, value);
        const outcome = await testCallThenable(value, then);
        if (outcome.status === "rejected") throw outcome.value;
        return await testResolveValue(outcome.value, seen, depth + 1, name);
    }

    function testTimeoutError(name, milliseconds) {
        return testError(
            "NEO_TEST_PENDING",
            `test "${name}" did not settle within ${milliseconds} ms`,
            "Error",
        );
    }

    function testTimeoutUnavailable(name) {
        return testError(
            "NEO_TEST_NO_TIMER",
            `cannot enforce the test timeout for "${name}": timer support is unavailable`,
            "Error",
        );
    }

    /**
     * Bound one assimilation chain by one timeout when host timers exist.
     *
     * `promise` is the native Promise returned by `testResolveValue`; it
     * fulfills with a non-thenable wrapper or rejects with the failure reason.
     * The observation attaches handlers through the host hook, so a subclass
     * with a throwing `then`, `constructor`, or `Symbol.species` is observed by
     * its real state instead of tripping a prototype getter.
     */
    function testWithTimeout(promise, milliseconds, name) {
        if (!hasTimers) {
            return promise;
        }
        return new NativePromise(function (resolve, reject) {
            let settled = false;
            let timer;
            try {
                timer = testCreateTimer(
                    function () {
                        if (settled) return;
                        settled = true;
                        reject(testTimeoutError(name, milliseconds));
                    },
                    milliseconds,
                    undefined,
                    false,
                    true,
                    false,
                );
            } catch {
                try {
                    observePromise(promise);
                } catch {
                    // The timer failure is the useful result.
                }
                reject(testTimeoutUnavailable(name));
                return;
            }
            let observation;
            try {
                observation = observePromise(promise);
            } catch (error) {
                try {
                    testCancelTimer(timer);
                } catch {
                    // The timer already fired or was cleaned up.
                }
                reject(error);
                return;
            }
            (async function () {
                try {
                    await observation.promise;
                    if (settled) return;
                    settled = true;
                    try {
                        testCancelTimer(timer);
                    } catch {
                        // The timer already fired or was cleaned up.
                    }
                    if (observation.status === OBSERVE_FULFILLED) {
                        // The wrapper is non-thenable; native resolution cannot
                        // assimilate it through a mutable `then`.
                        resolve(observation.value);
                    } else if (observation.status === OBSERVE_REJECTED) {
                        reject(observation.value);
                    } else {
                        reject(testObserveError(name, observation.error));
                    }
                } catch (error) {
                    if (settled) return;
                    settled = true;
                    reject(error);
                }
            })();
        });
    }

    async function testDrain(milliseconds) {
        if (!hasTimers) {
            await nativePromiseResolve(undefined);
            if (typeof testFlushUnhandled === "function") testFlushUnhandled();
            return;
        }
        const delay = milliseconds === undefined ? 1 : milliseconds;
        await new NativePromise(function (resolve) {
            try {
                testCreateTimer(
                    function () {
                        resolve(undefined);
                    },
                    delay,
                    undefined,
                    false,
                    true,
                    false,
                );
            } catch {
                resolve(undefined);
            }
        });
    }

    /**
     * Wait for a bounded number of event-loop turns and report whether the
     * runtime still sees pending work. `false` means the verdict is unsafe:
     * a detached timer, async op, dynamic import, or rejection event may
     * still change the result after the summary was built.
     */
    async function testSettle() {
        if (!hasTimers) {
            if (typeof testEventLoopHasMoreWork !== "function") {
                await testDrain();
                return true;
            }
            for (let round = 0; round < TEST_SETTLE_ROUNDS; round += 1) {
                await nativePromiseResolve(undefined);
                if (!testEventLoopHasMoreWork()) {
                    if (typeof testFlushUnhandled === "function") testFlushUnhandled();
                    return true;
                }
            }
            if (typeof testFlushUnhandled === "function") testFlushUnhandled();
            return false;
        }
        if (typeof testEventLoopHasMoreWork !== "function") {
            await testDrain();
            return true;
        }
        for (let round = 0; round < TEST_SETTLE_ROUNDS; round += 1) {
            await testDrain();
            if (!testEventLoopHasMoreWork()) return true;
        }
        return false;
    }

    function testIsBlank(name) {
        for (let index = 0; index < name.length; index += 1) {
            const character = name[index];
            if (
                character !== " " &&
                character !== "\t" &&
                character !== "\n" &&
                character !== "\r"
            ) {
                return false;
            }
        }
        return true;
    }

    function testRegistrationTimeout(options) {
        if (options === undefined) {
            return hasTimers ? TEST_DEFAULT_TIMEOUT_MS : 0;
        }
        if (options === null || typeof options !== "object") {
            throw new makeTypeError("test options must be an object");
        }
        const keys = objectKeys(options);
        for (let index = 0; index < keys.length; index += 1) {
            if (keys[index] !== "timeout") {
                throw new makeTypeError(`unknown test option \`${NativeString(keys[index])}\``);
            }
        }
        const timeout = options.timeout;
        if (timeout === undefined) {
            return hasTimers ? TEST_DEFAULT_TIMEOUT_MS : 0;
        }
        if (!isInteger(timeout) || timeout <= 0 || timeout > 2147483647) {
            throw makeRangeError("test timeout must be a positive integer of milliseconds");
        }
        if (!hasTimers) {
            throw testError(
                "NEO_TEST_NO_TIMER",
                "test `timeout` option is not supported in this runtime: timer support is unavailable",
                "Error",
            );
        }
        return timeout;
    }

    function testRunOptions(options) {
        if (options === undefined) return { throwOnFailure: true, settle: false };
        if (options === null || typeof options !== "object") {
            throw new makeTypeError("run options must be an object");
        }
        const keys = objectKeys(options);
        for (let index = 0; index < keys.length; index += 1) {
            if (keys[index] !== "throwOnFailure" && keys[index] !== "settle") {
                throw new makeTypeError(`unknown run option \`${NativeString(keys[index])}\``);
            }
        }
        const throwOnFailure = options.throwOnFailure;
        if (throwOnFailure !== undefined && typeof throwOnFailure !== "boolean") {
            throw new makeTypeError("run option throwOnFailure must be a boolean");
        }
        const settle = options.settle;
        if (settle !== undefined && typeof settle !== "boolean") {
            throw new makeTypeError("run option settle must be a boolean");
        }
        return {
            throwOnFailure: throwOnFailure === undefined ? true : throwOnFailure,
            settle: settle === undefined ? false : settle,
        };
    }

    function testSummaryObject() {
        const summary = { __proto__: null };
        defineField(summary, "total", testTotal);
        defineField(summary, "passed", testPassed);
        defineField(summary, "failed", testFailed);
        defineField(summary, "pending", testPending);
        defineField(summary, "unhandled", testUnhandled);
        defineField(summary, "unsettled", testUnsettled);
        const failures = [];
        for (let index = 0; index < testFailures.length; index += 1) {
            append(failures, testFailures[index]);
        }
        defineField(summary, "failures", failures);
        return summary;
    }

    function testPrintSummary() {
        const parts = [`${testTotal} test${testTotal === 1 ? "" : "s"}`];
        append(parts, `${testPassed} passed`);
        append(parts, `${testFailed} failed`);
        if (testPending > 0) append(parts, `${testPending} pending`);
        if (testUnhandled > 0) append(parts, `${testUnhandled} unhandled`);
        if (testUnsettled > 0) append(parts, `${testUnsettled} unsettled`);
        testPrintLine(`# ${arrayJoin(parts, ", ")}`);
    }

    function testRecordFailure(line) {
        append(testFailures, line);
    }

    function testUnhandledRejection(promise, reason) {
        try {
            testUnhandled += 1;
            const line = `unhandled rejection: ${testDescribeError(reason)}`;
            testRecordFailure(line);
            testPrintLine(`# ${line}`);
        } catch {
            // The count is already recorded, so the run still fails.
        }
        // Tell the host the rejection was observed. The runner reports it as a
        // failure; letting the process abort here would hide every later test.
        return true;
    }

    function testEnsureHandler() {
        if (testHandlerInstalled) return;
        testHandlerInstalled = true;
        try {
            if (typeof testSetUnhandledRejectionHandler === "function") {
                testSetUnhandledRejectionHandler(testUnhandledRejection);
            }
        } catch {
            // If the runtime has no handler slot, an unhandled rejection keeps
            // its normal behaviour rather than being silently discarded.
        }
    }

    /** Thrown by `run()` after one or more tests failed, were pending, or left an unhandled rejection. */
    class TestFailureError extends NativeError {
        constructor(message, failures) {
            super(message);
            defineField(this, "name", "TestFailureError");
            defineField(this, "code", "NEO_TEST_FAILURES");
            defineField(this, "failures", failures);
        }
    }

    /**
     * Register a test. Sync and async callbacks are both accepted; a returned
     * thenable is awaited. With no callback the test is registered as pending,
     * which fails the run. A generator function is rejected at registration.
     *
     * @throws {TypeError} for a bad name, callback, or options.
     * @throws {Error} with code `NEO_TEST_DURING_RUN` when called during a run.
     * @throws {Error} with code `NEO_TEST_LIMIT` past `MAX_TESTS`.
     */
    function test(name, fn, options) {
        if (arguments.length < 1 || arguments.length > 3) {
            throw new makeTypeError("test expects one to three arguments");
        }
        if (testRunning) {
            throw testError(
                "NEO_TEST_DURING_RUN",
                "cannot register a test while tests are running",
                "Error",
            );
        }
        if (typeof name !== "string" || testIsBlank(name)) {
            throw new makeTypeError("test name must be a non-empty string");
        }
        const pending = arguments.length === 1;
        let timeout = 0;
        if (!pending) {
            if (typeof fn !== "function") {
                throw new makeTypeError("test callback must be a function");
            }
            if (testFunctionKind(fn) === "generator") {
                throw testError(
                    "NEO_TEST_UNSUPPORTED",
                    `test "${testLabel(name)}" must be a sync or async function, not a generator function`,
                    "TypeError",
                );
            }
            timeout = testRegistrationTimeout(options);
        }
        if (testRegistered >= TEST_MAX) {
            throw testError(
                "NEO_TEST_LIMIT",
                `cannot register more than ${TEST_MAX} tests in one runtime`,
                "Error",
            );
        }
        testRegistered += 1;
        defineField(testQueue, testQueueLength, {
            name,
            fn: pending ? undefined : fn,
            pending,
            timeout,
        });
        testQueueLength += 1;
    }

    /**
     * Run every queued test in registration order and return cumulative counts.
     *
     * A failing test does not stop later tests. Failures, pending tests,
     * unhandled rejections, and an unsettled event loop are printed and
     * retained. `options.throwOnFailure` defaults to true and throws whenever
     * the cumulative report has any failure, including one retained from an
     * earlier call. The host passes false so it can read the report after a
     * failing run. `options.settle` defaults to false and is intended for the
     * host's final pass: a bounded event-loop settle check is only meaningful
     * after the entry module has finished, because a call made from top-level
     * await sees its own pending module evaluation. When host timers exist, a
     * callback that never settles is failed after its timeout (30 seconds by
     * default).
     */
    async function run(options) {
        const runOptions = testRunOptions(options);
        const throwOnFailure = runOptions.throwOnFailure;
        if (testRunning) {
            throw testError(
                "NEO_TEST_ALREADY_RUNNING",
                "tests are already running",
                "Error",
            );
        }
        testRunning = true;
        try {
            const start = testCursor;
            let runIndex = 0;
            for (let index = start; index < testQueueLength; index += 1) {
                const entry = testQueue[index];
                const name = testLabel(entry.name);
                runIndex += 1;
                testTotal += 1;
                if (entry.pending) {
                    testPending += 1;
                    const detail = `${name}: pending; no callback was supplied`;
                    testRecordFailure(detail);
                    testPrintLine(`not ok ${runIndex} - ${name} # PENDING`);
                    testPrintLine(`  ${detail}`);
                    continue;
                }
                try {
                    const result = entry.fn();
                    if (testGeneratorObject(result)) {
                        throw testError(
                            "NEO_TEST_UNSUPPORTED",
                            `test "${name}" returned a generator object; the body did not run`,
                            "TypeError",
                        );
                    }
                    await testWithTimeout(testResolveValue(result, [], 0, name), entry.timeout, name);
                    testPassed += 1;
                    testPrintLine(`ok ${runIndex} - ${name}`);
                } catch (error) {
                    testFailed += 1;
                    const detail = `${name}: ${testDescribeError(error)}`;
                    testRecordFailure(detail);
                    testPrintLine(`not ok ${runIndex} - ${name}`);
                    testPrintIndented(testDescribeError(error));
                    testPrintStack(error);
                }
            }
            testCursor = testQueueLength;
            if (runOptions.settle) {
                // Drain bounded event-loop work so a detached timer or async op
                // that was scheduled by a test gets a chance to settle or
                // reject. If the loop still reports work, the verdict is
                // unsafe.
                const settled = await testSettle();
                if (!settled && !testUnsettledRecorded) {
                    testUnsettledRecorded = true;
                    testUnsettled = 1;
                    const detail = hasTimers
                        ? `unsettled work: pending event-loop work remained after ${TEST_SETTLE_ROUNDS} ms`
                        : `unsettled work: pending job queue work remained after ${TEST_SETTLE_ROUNDS} rounds`;
                    testRecordFailure(detail);
                    testPrintLine(`# ${detail}`);
                }
            } else {
                // Deliver microtask/macrotask rejection events without claiming
                // to have observed all detached work. The host's final pass
                // uses `settle: true`.
                await testDrain();
            }
        } finally {
            testRunning = false;
        }
        const summary = testSummaryObject();
        testPrintSummary();
        if (
            throwOnFailure &&
            (testFailed > 0 || testPending > 0 || testUnhandled > 0 || testUnsettled > 0)
        ) {
            throw new TestFailureError(
                `${testFailed} failed, ${testPending} pending, ${testUnhandled} unhandled, ${testUnsettled} unsettled`,
                summary.failures,
            );
        }
        return summary;
    }

    const testModule = freeze({
        test,
        run,
        MAX_TESTS: TEST_MAX,
        TestFailureError,
    });

    return {
        module: testModule,
        ensureHandler: testEnsureHandler,
    };
})
