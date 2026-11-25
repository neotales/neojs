(function createVaelAssert(host) {
    "use strict";

    // `assert` is a small assertion surface inspired by Node's `assert/strict`
    // and Deno's `@std/assert`. It is an independent implementation informed by
    // the NeoLiteJS `neo:assert` module; it does not claim compatibility with
    // either and exports no aliases. In particular there is no `assertEquals`,
    // `deepEqual`, or `strictEqual`: structural equality is `equals` and
    // identity is `strictEquals`.
    //
    // `equals` is strict structural equality. Prototypes must match, own
    // enumerable string and symbol properties are compared, arrays also compare
    // length, and Dates, RegExps, Maps, Sets, ArrayBuffers, DataViews, typed
    // arrays, boxed primitives, and Errors compare their internal state as well
    // as own properties. Maps and Sets are compared without regard to iteration
    // order; functions and genuine native Promises, WeakMaps, WeakSets, and
    // WeakRefs compare by identity. `NaN` equals `NaN`, `-0` differs from `0`,
    // and cycles are handled. `strictEquals` is exactly `Object.is`: it never
    // compares two distinct objects structurally.
    //
    // `throws` and `doesNotThrow` take synchronous callbacks. A callback that
    // returns a genuine native Promise (its internal slot, not merely
    // `Promise.prototype`) is a `TypeError` that points at `rejects` or
    // `doesNotReject`, rather than a pass based on work that has not run.
    // `rejects` and `doesNotReject` return Promises and handle a synchronous
    // throw and a returned thenable, reading `then` once.
    //
    // Capture the intrinsics used by comparison, callback checks, and
    // reporting. Code in the same realm can replace these globals later; that
    // does not change whether an assertion passes or make the formatter throw.
    // This is not isolation from code that replaced them before this script ran.
    const makeTypeError = TypeError;
    const makeRegExp = RegExp;
    const makeUint8Array = Uint8Array;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const objectKeys = Object.keys;
    const arrayIsArray = Array.isArray;
    const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const objectIs = Object.is;
    const getOwnPropertySymbols = Object.getOwnPropertySymbols;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
    const hasOwnProperty = Object.prototype.hasOwnProperty;
    const stringify = JSON.stringify;
    const mathMin = Math.min;
    const arrayJoin = Function.prototype.call.bind(Array.prototype.join);
    const arrayBufferIsView = ArrayBuffer.isView;
    const NativeMap = Map;
    const NativeSet = Set;
    const NativeDate = Date;
    const NativeRegExp = RegExp;
    const NativeError = Error;
    const NativeBoolean = Boolean;
    const NativeNumber = Number;
    const NativeString = String;
    const NativeBigInt = typeof BigInt === "function" ? BigInt : null;
    const NativeSymbol = Symbol;
    const NativeArrayBuffer = ArrayBuffer;
    const NativeSharedArrayBuffer =
        typeof SharedArrayBuffer === "function" ? SharedArrayBuffer : null;
    const NativePromise = Promise;
    const NativeWeakMap = WeakMap;
    const NativeWeakSet = WeakSet;
    const NativeWeakRef = typeof WeakRef === "function" ? WeakRef : null;
    const NativeDataView = typeof DataView === "function" ? DataView : null;

    if (
        !host ||
        typeof host.isNativePromise !== "function" ||
        typeof host.observePromise !== "function"
    ) {
        throw new makeTypeError(
            "vael:assert host requires isNativePromise and observePromise hooks",
        );
    }

    // `Function.prototype.call` is captured once so a later replacement cannot
    // change an equality result or make a guard call the wrong function.
    const uncurry = Function.prototype.call.bind(Function.prototype.call);
    const functionHasInstance = Function.prototype[Symbol.hasInstance];

    const dateGetTime = NativeDate.prototype.getTime;
    const dateToISOString = NativeDate.prototype.toISOString;
    const regexpSource = getOwnPropertyDescriptor(NativeRegExp.prototype, "source").get;
    const regexpFlags = getOwnPropertyDescriptor(NativeRegExp.prototype, "flags").get;
    const mapSize = getOwnPropertyDescriptor(NativeMap.prototype, "size").get;
    const mapForEach = NativeMap.prototype.forEach;
    const mapDelete = NativeMap.prototype.delete;
    const setSize = getOwnPropertyDescriptor(NativeSet.prototype, "size").get;
    const setForEach = NativeSet.prototype.forEach;
    const setDelete = NativeSet.prototype.delete;
    const booleanValueOf = NativeBoolean.prototype.valueOf;
    const numberValueOf = NativeNumber.prototype.valueOf;
    const stringValueOf = NativeString.prototype.valueOf;
    const bigintValueOf = NativeBigInt ? NativeBigInt.prototype.valueOf : null;
    const symbolValueOf = NativeSymbol.prototype.valueOf;
    const arrayBufferByteLength = getOwnPropertyDescriptor(
        NativeArrayBuffer.prototype,
        "byteLength",
    ).get;
    const sharedArrayBufferByteLength = NativeSharedArrayBuffer
        ? getOwnPropertyDescriptor(NativeSharedArrayBuffer.prototype, "byteLength").get
        : null;
    const dataViewBuffer = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "buffer").get
        : null;
    const dataViewByteOffset = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "byteOffset").get
        : null;
    const dataViewByteLength = NativeDataView
        ? getOwnPropertyDescriptor(NativeDataView.prototype, "byteLength").get
        : null;
    const TypedArrayPrototype = getPrototypeOf(makeUint8Array.prototype);
    const typedArrayTag = getOwnPropertyDescriptor(
        TypedArrayPrototype,
        NativeSymbol.toStringTag,
    ).get;
    const typedArrayLength = getOwnPropertyDescriptor(TypedArrayPrototype, "length").get;
    const typedArrayBuffer = getOwnPropertyDescriptor(TypedArrayPrototype, "buffer").get;
    const typedArrayByteOffset = getOwnPropertyDescriptor(
        TypedArrayPrototype,
        "byteOffset",
    ).get;

    // Engine-backed checks look at the internal Promise slot. They do not read
    // `then`, consult a Symbol.hasInstance, or run a proxy trap, so an object
    // that merely inherits from Promise.prototype is not a Promise.
    const coreIsPromise = host.isNativePromise;
    const observePromiseOp = host.observePromise;
    // Status and sentinel values are symbols. They are primitives, so native
    // Promise resolution never reads an inherited `Object.prototype.then` from
    // one, and they are never exposed to caller code.
    const NOT_THENABLE = NativeSymbol("assert.not-thenable");
    const OBSERVE_PENDING = NativeSymbol("assert.pending");
    const OBSERVE_FULFILLED = NativeSymbol("assert.fulfilled");
    const OBSERVE_REJECTED = NativeSymbol("assert.rejected");
    const OBSERVE_FAILED = NativeSymbol("assert.inspection-failed");
    const captureStackTrace = NativeError.captureStackTrace;

    const MAX_TEXT = 400;
    const MAX_ITEMS = 50;
    const MAX_DEPTH = 8;
    const MAX_NAME = 64;
    const INDEX_KEY = /^(0|[1-9][0-9]*)$/;
    const IDENTIFIER_KEY = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

    // Create an own public field with the attributes plain assignment would
    // give it. An inherited setter for the field name must not run, and the
    // descriptor must not inherit Object.prototype.get or .set.
    function defineField(object, key, value) {
        defineProperty(object, key, {
            __proto__: null,
            value,
            enumerable: true,
            configurable: true,
            writable: true,
        });
    }

    // Create own index properties rather than assigning through an inherited
    // setter on Array.prototype.
    function defineIndex(array, index, value) {
        defineField(array, index, value);
    }

    function append(array, value) {
        defineIndex(array, array.length, value);
    }

    function contains(array, value) {
        for (let index = 0; index < array.length; index += 1) {
            if (array[index] === value) return true;
        }
        return false;
    }

    /** Thrown when an assertion fails. */
    class AssertionError extends NativeError {
        constructor(message, options) {
            super(message);
            // Own data properties: an inherited setter for a field name must
            // not replace this error or drop its code. The code is
            // vaeljs-specific; it is not Node's `ERR_ASSERTION`.
            defineField(this, "name", "AssertionError");
            defineField(this, "code", "VAEL_ASSERT");
            const details = options || {};
            defineField(this, "operator", details.operator || "assert");
            if ("actual" in details) defineField(this, "actual", details.actual);
            if ("expected" in details) defineField(this, "expected", details.expected);
            if (typeof captureStackTrace === "function") {
                captureStackTrace(this, AssertionError);
            }
        }
    }

    function checkMessage(message) {
        if (message !== undefined && typeof message !== "string") {
            throw new makeTypeError("message must be a string");
        }
    }

    function checkArity(args, name, minimum, maximum) {
        if (args.length < minimum || args.length > maximum) {
            const expected =
                minimum === maximum ? NativeString(minimum) : `${minimum} to ${maximum}`;
            throw new makeTypeError(
                `${name} expects ${expected} argument${maximum === 1 ? "" : "s"}, got ${args.length}`,
            );
        }
    }

    function checkCallback(callback) {
        if (typeof callback !== "function") {
            throw new makeTypeError("callback must be a function");
        }
    }

    function isObjectLike(value) {
        return value !== null && (typeof value === "object" || typeof value === "function");
    }

    // OrdinaryHasInstance, called directly so a caller's own Symbol.hasInstance
    // hook cannot turn a wrong error class into a pass.
    function hasBrand(value, constructor) {
        try {
            return uncurry(functionHasInstance, constructor, value);
        } catch {
            return false;
        }
    }

    // A genuine native Promise check backed by the engine's internal slot. It
    // does not read a property, consult the prototype chain, or run a proxy
    // trap, so `Object.create(Promise.prototype)` is not a Promise.
    function isNativePromise(value) {
        return coreIsPromise(value);
    }

    /**
     * Observe a genuine native Promise without trusting its `then`.
     *
     * The host's internal Promise observation attaches the handlers without
     * reading an own or inherited `then` and without creating a derived promise
     * through `Symbol.species`. If the host declines the observation, the
     * result is `OBSERVE_FAILED` instead of a fulfillment or rejection. The
     * wrapper promise always resolves with `undefined`; it never resolves with
     * an object that a polluted `Object.prototype.then` could assimilate.
     */
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

    /**
     * Assimilate a non-native thenable by reading `then` exactly once.
     *
     * Native `resolve`/`reject` are passed to `then`, so nested thenables and
     * rejection handling follow ordinary Promise resolution. A plain value
     * resolves with the non-thenable `NOT_THENABLE` symbol, which cannot pick
     * up an inherited `Object.prototype.then`.
     */
    function assimilateThenable(value) {
        return new NativePromise(function (resolve, reject) {
            if (!isObjectLike(value)) {
                resolve(NOT_THENABLE);
                return;
            }
            let then;
            try {
                then = value.then;
            } catch (error) {
                reject(error);
                return;
            }
            if (typeof then !== "function") {
                resolve(NOT_THENABLE);
                return;
            }
            try {
                uncurry(then, value, resolve, reject);
            } catch (error) {
                reject(error);
            }
        });
    }

    // Consume a native Promise that a synchronous assertion is refusing, so a
    // later rejection is reported as handled rather than as a second failure.
    // The same host-backed observer avoids a user `then` and `Symbol.species`.
    function settlePromise(promise) {
        try {
            observePromiseOp(promise, function () {}, function () {});
        } catch {
            // The TypeError from the synchronous guard is still the useful
            // result if the host declines the observation.
        }
    }

    function withMessage(message, detail) {
        return message === undefined ? detail : `${message}: ${detail}`;
    }

    function truncate(text) {
        const value = NativeString(text);
        if (value.length <= MAX_TEXT) return value;
        return (
            stringSlice(value, 0, MAX_TEXT) +
            `... (${value.length - MAX_TEXT} more characters)`
        );
    }

    function quote(text) {
        return stringify(truncate(text));
    }

    function formatKey(key) {
        if (typeof key === "symbol") return `[${NativeString(key)}]`;
        if (regexpExec(INDEX_KEY, key) !== null) return `[${key}]`;
        return regexpExec(IDENTIFIER_KEY, key) !== null ? `.${key}` : `[${quote(key)}]`;
    }

    /** Own enumerable string and symbol keys, in property order. */
    function ownKeys(value) {
        const keys = objectKeys(value);
        const symbols = getOwnPropertySymbols(value);
        for (let index = 0; index < symbols.length; index += 1) {
            if (uncurry(propertyIsEnumerable, value, symbols[index])) {
                append(keys, symbols[index]);
            }
        }
        return keys;
    }

    function constructorName(value) {
        const prototype = getPrototypeOf(value);
        if (prototype === null) return "null-prototype";
        let constructor;
        try {
            constructor = prototype.constructor;
        } catch {
            return "";
        }
        const name = constructor && typeof constructor.name === "string" ? constructor.name : "";
        if (name === "Object" || name.length === 0) return "";
        return name.length <= MAX_NAME ? name : `${stringSlice(name, 0, MAX_NAME)}...`;
    }

    function safeMessage(error) {
        try {
            if (error && typeof error.message === "string") return error.message;
            return NativeString(error);
        } catch {
            return "unprintable error";
        }
    }

    function formatObject(value, depth, seen) {
        if (arrayIsArray(value)) {
            const parts = [];
            const length = mathMin(value.length, MAX_ITEMS);
            for (let index = 0; index < length; index += 1) {
                append(
                    parts,
                    uncurry(hasOwnProperty, value, index)
                        ? formatValue(value[index], depth + 1, seen)
                        : "<empty>",
                );
            }
            if (value.length > MAX_ITEMS) append(parts, "...");
            return `[${arrayJoin(parts, ", ")}]`;
        }

        if (hasBrand(value, NativeDate)) {
            const time = uncurry(dateGetTime, value);
            return time === time ? `Date(${uncurry(dateToISOString, value)})` : "Invalid Date";
        }
        if (hasBrand(value, NativeRegExp)) {
            return `/${uncurry(regexpSource, value)}/${uncurry(regexpFlags, value)}`;
        }
        if (hasBrand(value, NativeMap)) {
            const parts = [];
            let count = 0;
            uncurry(mapForEach, value, function (entryValue, key) {
                if (count >= MAX_ITEMS) return;
                count += 1;
                append(
                    parts,
                    `${formatValue(key, depth + 1, seen)} => ${formatValue(
                        entryValue,
                        depth + 1,
                        seen,
                    )}`,
                );
            });
            const size = uncurry(mapSize, value);
            if (size > MAX_ITEMS) append(parts, "...");
            return `Map(${size}) {${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
        }
        if (hasBrand(value, NativeSet)) {
            const parts = [];
            let count = 0;
            uncurry(setForEach, value, function (entryValue) {
                if (count >= MAX_ITEMS) return;
                count += 1;
                append(parts, formatValue(entryValue, depth + 1, seen));
            });
            const size = uncurry(setSize, value);
            if (size > MAX_ITEMS) append(parts, "...");
            return `Set(${size}) {${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
        }
        if (hasBrand(value, NativeArrayBuffer)) {
            return `ArrayBuffer(${uncurry(arrayBufferByteLength, value)})`;
        }
        if (NativeSharedArrayBuffer && hasBrand(value, NativeSharedArrayBuffer)) {
            return `SharedArrayBuffer(${uncurry(sharedArrayBufferByteLength, value)})`;
        }
        if (NativeDataView && hasBrand(value, NativeDataView)) {
            return `DataView(${uncurry(dataViewByteLength, value)})`;
        }
        if (arrayBufferIsView(value)) {
            const tag = uncurry(typedArrayTag, value);
            const length = uncurry(typedArrayLength, value);
            const parts = [];
            const count = mathMin(length, MAX_ITEMS);
            for (let index = 0; index < count; index += 1) {
                append(parts, formatValue(value[index], depth + 1, seen));
            }
            if (length > MAX_ITEMS) append(parts, "...");
            return `${tag}(${length}) [${arrayJoin(parts, ", ")}]`;
        }
        if (hasBrand(value, NativeError)) {
            let name = "Error";
            let message = "<message unreadable>";
            try {
                if (typeof value.name === "string" && value.name) name = value.name;
            } catch {
                // Keep the fallback name.
            }
            try {
                message = truncate(value.message);
            } catch {
                // Keep the fallback message.
            }
            return `${name}: ${message}`;
        }
        if (hasBrand(value, NativeBoolean)) {
            return `[Boolean: ${formatValue(uncurry(booleanValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeNumber)) {
            return `[Number: ${formatValue(uncurry(numberValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeString)) {
            return `[String: ${formatValue(uncurry(stringValueOf, value), depth + 1, seen)}]`;
        }
        if (NativeBigInt && hasBrand(value, NativeBigInt)) {
            return `[BigInt: ${formatValue(uncurry(bigintValueOf, value), depth + 1, seen)}]`;
        }
        if (hasBrand(value, NativeSymbol)) {
            return `[Symbol: ${formatValue(uncurry(symbolValueOf, value), depth + 1, seen)}]`;
        }
        if (isNativePromise(value)) return "Promise { <state unknown> }";
        if (hasBrand(value, NativeWeakMap)) return "WeakMap { <items unknown> }";
        if (hasBrand(value, NativeWeakSet)) return "WeakSet { <items unknown> }";
        if (NativeWeakRef && hasBrand(value, NativeWeakRef)) return "WeakRef { <target unknown> }";

        const name = constructorName(value);
        const keys = ownKeys(value);
        const parts = [];
        for (let index = 0; index < keys.length && index < MAX_ITEMS; index += 1) {
            const key = keys[index];
            let text;
            try {
                text = `${formatKey(key)}: ${formatValue(value[key], depth + 1, seen)}`;
            } catch (error) {
                text = `${formatKey(key)}: [threw ${truncate(safeMessage(error))}]`;
            }
            append(parts, text);
        }
        if (keys.length > MAX_ITEMS) append(parts, "...");
        return `${name ? `${name} ` : ""}{${parts.length ? ` ${arrayJoin(parts, ", ")} ` : ""}}`;
    }

    /**
     * Best-effort value formatting for assertion messages. Getters are invoked,
     * cycles print as `[Circular]`, and output is truncated by depth, item
     * count, and total text length. This never throws: a hostile accessor or a
     * mutated prototype yields a fallback string, so the assertion still
     * reports its failure.
     */
    function formatValue(value, depth, seen) {
        try {
            return formatValueUnsafe(value, depth, seen);
        } catch {
            return "[unprintable value]";
        }
    }

    function formatValueUnsafe(value, depth, seen) {
        const level = depth || 0;
        const path = seen || [];
        if (value === null) return "null";
        const kind = typeof value;
        if (kind === "string") return quote(value);
        if (kind === "number") return objectIs(value, -0) ? "-0" : NativeString(value);
        if (kind === "bigint") return `${NativeString(value)}n`;
        if (kind === "boolean" || kind === "undefined") return NativeString(value);
        if (kind === "symbol") return NativeString(value);
        if (kind === "function") {
            let name;
            try {
                name = value.name;
            } catch {
                name = "";
            }
            return typeof name === "string" && name
                ? `[Function: ${truncate(name)}]`
                : "[Function (anonymous)]";
        }
        if (contains(path, value)) return "[Circular]";
        if (level >= MAX_DEPTH) return arrayIsArray(value) ? "[Array]" : "[Object]";
        append(path, value);
        try {
            return formatObject(value, level, path);
        } catch (error) {
            return `[uninspectable: ${truncate(safeMessage(error))}]`;
        } finally {
            path.length -= 1;
        }
    }

    function mismatch(path, reason, actual, expected) {
        return { path, reason, actual, expected };
    }

    function markPair(seen, actual, expected) {
        let expectedSet = mapGet(seen.map, actual);
        if (expectedSet === undefined) {
            expectedSet = new NativeSet();
            mapSet(seen.map, actual, expectedSet);
        }
        if (setHas(expectedSet, expected)) return true;
        setAdd(expectedSet, expected);
        append(seen.log, [actual, expected]);
        return false;
    }

    const mapGet = Function.prototype.call.bind(NativeMap.prototype.get);
    const mapSet = Function.prototype.call.bind(NativeMap.prototype.set);
    const setAdd = Function.prototype.call.bind(NativeSet.prototype.add);
    const setHas = Function.prototype.call.bind(NativeSet.prototype.has);

    function rollback(seen, mark) {
        while (seen.log.length > mark) {
            const pair = seen.log[seen.log.length - 1];
            seen.log.length -= 1;
            const expectedSet = mapGet(seen.map, pair[0]);
            if (expectedSet !== undefined) {
                uncurry(setDelete, expectedSet, pair[1]);
                if (uncurry(setSize, expectedSet) === 0) {
                    uncurry(mapDelete, seen.map, pair[0]);
                }
            }
        }
    }

    /** Compare two values under the shared cycle-tracking state, rolling back on failure. */
    function sameValue(actual, expected, path, seen) {
        const mark = seen.log.length;
        if (findMismatch(actual, expected, path, seen) === null) return true;
        rollback(seen, mark);
        return false;
    }

    /**
     * Compare a Map key and value as one candidate, so a matching key whose
     * value differs leaves no cycle marks behind for the next candidate.
     */
    function sameEntry(actualEntry, expectedEntry, path, seen) {
        const mark = seen.log.length;
        if (
            findMismatch(actualEntry[0], expectedEntry[0], `${path}<Map key>`, seen) !== null ||
            findMismatch(actualEntry[1], expectedEntry[1], `${path}<Map value>`, seen) !== null
        ) {
            rollback(seen, mark);
            return false;
        }
        return true;
    }

    function kindOf(value) {
        if (arrayIsArray(value)) return "array";
        if (arrayBufferIsView(value)) {
            return NativeDataView && hasBrand(value, NativeDataView) ? "dataview" : "typedarray";
        }
        if (hasBrand(value, NativeDate)) return "date";
        if (hasBrand(value, NativeRegExp)) return "regexp";
        if (hasBrand(value, NativeMap)) return "map";
        if (hasBrand(value, NativeSet)) return "set";
        if (hasBrand(value, NativeArrayBuffer)) return "arraybuffer";
        if (NativeSharedArrayBuffer && hasBrand(value, NativeSharedArrayBuffer)) {
            return "sharedarraybuffer";
        }
        if (hasBrand(value, NativeBoolean)) return "boolean";
        if (hasBrand(value, NativeNumber)) return "number";
        if (hasBrand(value, NativeString)) return "string";
        if (NativeBigInt && hasBrand(value, NativeBigInt)) return "bigint";
        if (hasBrand(value, NativeSymbol)) return "symbol";
        if (hasBrand(value, NativeError)) return "error";
        if (isNativePromise(value)) return "promise";
        if (hasBrand(value, NativeWeakMap)) return "weakmap";
        if (hasBrand(value, NativeWeakSet)) return "weakset";
        if (NativeWeakRef && hasBrand(value, NativeWeakRef)) return "weakref";
        return "object";
    }

    function bytesEqual(actual, expected) {
        if (actual.length !== expected.length) return false;
        for (let index = 0; index < actual.length; index += 1) {
            if (actual[index] !== expected[index]) return false;
        }
        return true;
    }

    function bufferBytes(value, length, byteOffset) {
        return new makeUint8Array(value, byteOffset || 0, length);
    }

    /**
     * Return the first structural difference, or `null` when the values are
     * deeply equal. `path` is a readable location in `actual`.
     */
    function findMismatch(actual, expected, path, seen) {
        if (objectIs(actual, expected)) return null;
        const actualObject = isObjectLike(actual);
        const expectedObject = isObjectLike(expected);
        if (!actualObject || !expectedObject) {
            return actualObject !== expectedObject
                ? mismatch(path, "an object and a primitive are never equal", actual, expected)
                : mismatch(path, "values differ", actual, expected);
        }
        if (typeof actual === "function" || typeof expected === "function") {
            return mismatch(path, "functions are compared by identity", actual, expected);
        }
        if (markPair(seen, actual, expected)) return null;
        let actualPrototype;
        let expectedPrototype;
        try {
            actualPrototype = getPrototypeOf(actual);
            expectedPrototype = getPrototypeOf(expected);
        } catch {
            return mismatch(path, "a prototype could not be read", actual, expected);
        }
        if (actualPrototype !== expectedPrototype) {
            return mismatch(path, "prototypes differ", actual, expected);
        }

        let actualKind;
        let expectedKind;
        try {
            actualKind = kindOf(actual);
            expectedKind = kindOf(expected);
        } catch {
            return mismatch(path, "a value could not be inspected", actual, expected);
        }
        if (actualKind !== expectedKind) {
            return mismatch(
                path,
                `values are different kinds (${actualKind} and ${expectedKind})`,
                actual,
                expected,
            );
        }

        try {
            if (actualKind === "array") {
                if (actual.length !== expected.length) {
                    return mismatch(path, "array lengths differ", actual, expected);
                }
            } else if (actualKind === "date") {
                if (!objectIs(uncurry(dateGetTime, actual), uncurry(dateGetTime, expected))) {
                    return mismatch(path, "dates differ", actual, expected);
                }
            } else if (actualKind === "regexp") {
                if (
                    uncurry(regexpSource, actual) !== uncurry(regexpSource, expected) ||
                    uncurry(regexpFlags, actual) !== uncurry(regexpFlags, expected) ||
                    !objectIs(actual.lastIndex, expected.lastIndex)
                ) {
                    return mismatch(path, "regular expressions differ", actual, expected);
                }
            } else if (actualKind === "map") {
                const actualSize = uncurry(mapSize, actual);
                if (actualSize !== uncurry(mapSize, expected)) {
                    return mismatch(path, "Map sizes differ", actual, expected);
                }
                const expectedEntries = [];
                uncurry(mapForEach, expected, function (entryValue, entryKey) {
                    append(expectedEntries, [entryKey, entryValue]);
                });
                const used = [];
                for (let index = 0; index < expectedEntries.length; index += 1) {
                    append(used, false);
                }
                let missing = false;
                uncurry(mapForEach, actual, function (entryValue, entryKey) {
                    if (missing) return;
                    for (let index = 0; index < expectedEntries.length; index += 1) {
                        if (used[index] === true) continue;
                        if (sameEntry([entryKey, entryValue], expectedEntries[index], path, seen)) {
                            defineIndex(used, index, true);
                            return;
                        }
                    }
                    missing = true;
                });
                if (missing) return mismatch(path, "Map entries differ", actual, expected);
            } else if (actualKind === "set") {
                const actualSize = uncurry(setSize, actual);
                if (actualSize !== uncurry(setSize, expected)) {
                    return mismatch(path, "Set sizes differ", actual, expected);
                }
                const expectedValues = [];
                uncurry(setForEach, expected, function (entryValue) {
                    append(expectedValues, entryValue);
                });
                const used = [];
                for (let index = 0; index < expectedValues.length; index += 1) {
                    append(used, false);
                }
                let missing = false;
                uncurry(setForEach, actual, function (entryValue) {
                    if (missing) return;
                    for (let index = 0; index < expectedValues.length; index += 1) {
                        if (used[index] === true) continue;
                        if (
                            sameValue(
                                entryValue,
                                expectedValues[index],
                                `${path}<Set value>`,
                                seen,
                            )
                        ) {
                            defineIndex(used, index, true);
                            return;
                        }
                    }
                    missing = true;
                });
                if (missing) return mismatch(path, "Set values differ", actual, expected);
            } else if (actualKind === "arraybuffer" || actualKind === "sharedarraybuffer") {
                const actualLength =
                    actualKind === "arraybuffer"
                        ? uncurry(arrayBufferByteLength, actual)
                        : uncurry(sharedArrayBufferByteLength, actual);
                const expectedLength =
                    actualKind === "arraybuffer"
                        ? uncurry(arrayBufferByteLength, expected)
                        : uncurry(sharedArrayBufferByteLength, expected);
                if (actualLength !== expectedLength) {
                    return mismatch(path, "buffer byte lengths differ", actual, expected);
                }
                if (!bytesEqual(bufferBytes(actual, actualLength), bufferBytes(expected, expectedLength))) {
                    return mismatch(path, "buffer bytes differ", actual, expected);
                }
            } else if (actualKind === "dataview") {
                const actualLength = uncurry(dataViewByteLength, actual);
                const expectedLength = uncurry(dataViewByteLength, expected);
                if (actualLength !== expectedLength) {
                    return mismatch(path, "DataView byte lengths differ", actual, expected);
                }
                const actualBytes = bufferBytes(
                    uncurry(dataViewBuffer, actual),
                    actualLength,
                    uncurry(dataViewByteOffset, actual),
                );
                const expectedBytes = bufferBytes(
                    uncurry(dataViewBuffer, expected),
                    expectedLength,
                    uncurry(dataViewByteOffset, expected),
                );
                if (!bytesEqual(actualBytes, expectedBytes)) {
                    return mismatch(path, "DataView bytes differ", actual, expected);
                }
            } else if (actualKind === "typedarray") {
                if (uncurry(typedArrayTag, actual) !== uncurry(typedArrayTag, expected)) {
                    return mismatch(path, "typed array types differ", actual, expected);
                }
                const actualLength = uncurry(typedArrayLength, actual);
                if (actualLength !== uncurry(typedArrayLength, expected)) {
                    return mismatch(path, "typed array lengths differ", actual, expected);
                }
                for (let index = 0; index < actualLength; index += 1) {
                    const elementMismatch = findMismatch(
                        actual[index],
                        expected[index],
                        `${path}[${index}]`,
                        seen,
                    );
                    if (elementMismatch !== null) return elementMismatch;
                }
            } else if (
                actualKind === "boolean" ||
                actualKind === "number" ||
                actualKind === "string" ||
                actualKind === "bigint" ||
                actualKind === "symbol"
            ) {
                const valueOf =
                    actualKind === "boolean"
                        ? booleanValueOf
                        : actualKind === "number"
                          ? numberValueOf
                          : actualKind === "string"
                            ? stringValueOf
                            : actualKind === "bigint"
                              ? bigintValueOf
                              : symbolValueOf;
                if (!objectIs(uncurry(valueOf, actual), uncurry(valueOf, expected))) {
                    return mismatch(path, "boxed primitive values differ", actual, expected);
                }
            } else if (actualKind === "error") {
                const actualName = actual.name;
                const expectedName = expected.name;
                const actualMessage = actual.message;
                const expectedMessage = expected.message;
                if (actualName !== expectedName) {
                    return mismatch(path, "error names differ", actual, expected);
                }
                if (actualMessage !== expectedMessage) {
                    return mismatch(path, "error messages differ", actual, expected);
                }
            } else if (
                actualKind === "promise" ||
                actualKind === "weakmap" ||
                actualKind === "weakset" ||
                actualKind === "weakref"
            ) {
                return mismatch(
                    path,
                    "Promise, WeakMap, WeakSet, and WeakRef values are compared by identity",
                    actual,
                    expected,
                );
            }
        } catch {
            return mismatch(path, `a ${actualKind} value could not be inspected`, actual, expected);
        }

        let actualKeys;
        let expectedKeys;
        try {
            actualKeys = ownKeys(actual);
            expectedKeys = ownKeys(expected);
        } catch {
            return mismatch(path, "own properties could not be read", actual, expected);
        }
        if (actualKeys.length !== expectedKeys.length) {
            return mismatch(
                path,
                "objects have different numbers of own enumerable properties",
                actual,
                expected,
            );
        }
        for (let index = 0; index < actualKeys.length; index += 1) {
            const key = actualKeys[index];
            let present;
            try {
                present = uncurry(propertyIsEnumerable, expected, key);
            } catch {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    "own properties could not be read",
                    actual,
                    expected,
                );
            }
            if (!present) {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    `property ${formatKey(key)} is missing`,
                    actual,
                    expected,
                );
            }
            let actualValue;
            let expectedValue;
            try {
                actualValue = actual[key];
                expectedValue = expected[key];
            } catch {
                return mismatch(
                    `${path}${formatKey(key)}`,
                    "a property could not be read",
                    actual,
                    expected,
                );
            }
            const propertyMismatch = findMismatch(
                actualValue,
                expectedValue,
                `${path}${formatKey(key)}`,
                seen,
            );
            if (propertyMismatch !== null) return propertyMismatch;
        }
        return null;
    }

    function describeMismatch(difference) {
        return (
            `values are not equal: ${difference.reason} at ${difference.path}` +
            `\nactual: ${formatValue(difference.actual)}` +
            `\nexpected: ${formatValue(difference.expected)}`
        );
    }

    function regexpText(regexp) {
        return `/${uncurry(regexpSource, regexp)}/${uncurry(regexpFlags, regexp)}`;
    }

    function checkExpected(expected) {
        if (expected === undefined || expected === null) return;
        if (hasBrand(expected, NativeRegExp)) {
            try {
                uncurry(regexpSource, expected);
                uncurry(regexpFlags, expected);
            } catch {
                throw new makeTypeError("expected must be a real RegExp, an error constructor, or omitted");
            }
            return;
        }
        if (typeof expected === "function") {
            let prototype;
            try {
                prototype = expected.prototype;
            } catch {
                prototype = undefined;
            }
            if (prototype !== null && typeof prototype === "object") return;
            throw new makeTypeError(
                "expected must be an error constructor, a RegExp, or omitted",
            );
        }
        throw new makeTypeError("expected must be an error constructor, a RegExp, or omitted");
    }

    function checkThrown(thrown, expected, message, operator) {
        if (expected === undefined || expected === null) return thrown;
        if (hasBrand(expected, NativeRegExp)) {
            const matcher = makeRegExp(uncurry(regexpSource, expected), uncurry(regexpFlags, expected));
            let text;
            try {
                text = hasBrand(thrown, NativeError)
                    ? NativeString(thrown.message)
                    : NativeString(thrown);
            } catch {
                text = "<unprintable thrown value>";
            }
            if (regexpExec(matcher, text) === null) {
                throw new AssertionError(
                    withMessage(
                        message,
                        `expected the thrown message to match ${regexpText(expected)}, ` +
                            `but it was ${formatValue(text)}`,
                    ),
                    { actual: text, expected, operator },
                );
            }
            return thrown;
        }
        if (!hasBrand(thrown, expected)) {
            const name =
                typeof expected.name === "string" && expected.name
                    ? expected.name
                    : "the expected error";
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function to throw an instance of ${name}, ` +
                        `but it threw ${formatValue(thrown)}`,
                ),
                { actual: thrown, expected, operator },
            );
        }
        return thrown;
    }

    /**
     * Throw unless `value` is truthy.
     *
     * @param {unknown} value
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `value` is falsy.
     */
    function assert(value, message) {
        checkArity(arguments, "assert", 1, 2);
        checkMessage(message);
        if (value) return;
        throw new AssertionError(
            withMessage(message, `expected a truthy value, got ${formatValue(value)}`),
            { actual: value, expected: true, operator: "assert" },
        );
    }

    /**
     * Throw unless the values are structurally equal.
     *
     * Equality is strict structural equality, not `==`: prototypes must match,
     * `NaN` equals `NaN`, `-0` differs from `0`, objects are compared by their
     * own enumerable properties and their built-in state, and cycles are
     * handled. See the module header for the complete rules.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when the values are not structurally equal.
     */
    function equals(actual, expected, message) {
        checkArity(arguments, "assert.equals", 2, 3);
        checkMessage(message);
        const difference = findMismatch(actual, expected, "$", {
            map: new NativeMap(),
            log: [],
        });
        if (difference === null) return;
        throw new AssertionError(withMessage(message, describeMismatch(difference)), {
            actual,
            expected,
            operator: "equals",
        });
    }

    /**
     * Throw when the values are structurally equal.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when the values are structurally equal.
     */
    function notEquals(actual, expected, message) {
        checkArity(arguments, "assert.notEquals", 2, 3);
        checkMessage(message);
        const difference = findMismatch(actual, expected, "$", {
            map: new NativeMap(),
            log: [],
        });
        if (difference !== null) return;
        throw new AssertionError(
            withMessage(
                message,
                `expected values to differ, but both are ${formatValue(actual)}`,
            ),
            { actual, expected, operator: "notEquals" },
        );
    }

    /**
     * Throw unless the values are the same value under `Object.is`.
     *
     * This is identity, not structural equality: two distinct objects are
     * never equal, `NaN` equals `NaN`, and `-0` differs from `0`.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `Object.is(actual, expected)` is false.
     */
    function strictEquals(actual, expected, message) {
        checkArity(arguments, "assert.strictEquals", 2, 3);
        checkMessage(message);
        if (objectIs(actual, expected)) return;
        throw new AssertionError(
            withMessage(
                message,
                "expected values to be identical under Object.is" +
                    `\nactual: ${formatValue(actual)}` +
                    `\nexpected: ${formatValue(expected)}`,
            ),
            { actual, expected, operator: "strictEquals" },
        );
    }

    /**
     * Throw when the values are the same value under `Object.is`.
     *
     * @param {unknown} actual
     * @param {unknown} expected
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} when `Object.is(actual, expected)` is true.
     */
    function notStrictEquals(actual, expected, message) {
        checkArity(arguments, "assert.notStrictEquals", 2, 3);
        checkMessage(message);
        if (!objectIs(actual, expected)) return;
        throw new AssertionError(
            withMessage(
                message,
                `expected values to differ under Object.is, but both are ${formatValue(actual)}`,
            ),
            { actual, expected, operator: "notStrictEquals" },
        );
    }

    /**
     * Throw an `AssertionError` unconditionally.
     *
     * @param {string} [message]
     * @throws {TypeError} when `message` is not a string or the argument count is wrong.
     * @throws {AssertionError} always.
     */
    function fail(message) {
        checkArity(arguments, "assert.fail", 0, 1);
        checkMessage(message);
        throw new AssertionError(message === undefined ? "failed" : message, {
            operator: "fail",
        });
    }

    /**
     * Throw unless calling `callback` throws. Returns the thrown value.
     *
     * `expected` is an error constructor checked with the intrinsic
     * `instanceof`, a `RegExp` tested against the thrown error's message (or
     * the string form of a non-Error value), or omitted to accept any throw. A
     * string second argument is a `TypeError`, not a message; pass the message
     * third. The callback must be synchronous: a native Promise return is a
     * `TypeError` that points at `rejects`.
     *
     * @param {() => unknown} callback
     * @param {Function|RegExp|null} [expected]
     * @param {string} [message]
     * @throws {TypeError} for a bad argument or a Promise-returning callback.
     * @throws {AssertionError} when the callback does not throw or throws the wrong value.
     */
    function throws(callback, expected, message) {
        checkArity(arguments, "assert.throws", 1, 3);
        checkMessage(message);
        checkCallback(callback);
        checkExpected(expected);
        let didThrow = false;
        let thrown;
        let returned;
        try {
            returned = callback();
        } catch (error) {
            didThrow = true;
            thrown = error;
        }
        if (!didThrow) {
            if (isNativePromise(returned)) {
                settlePromise(returned);
                throw new makeTypeError(
                    "assert.throws expects a synchronous function; " +
                        "use assert.rejects for a Promise-returning function",
                );
            }
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function to throw, but it returned ${formatValue(returned)}`,
                ),
                { actual: returned, operator: "throws" },
            );
        }
        return checkThrown(thrown, expected, message, "throws");
    }

    /**
     * Throw when calling `callback` throws, and otherwise return its result.
     *
     * The callback must be synchronous: a genuine native Promise return is a
     * `TypeError` that points at `doesNotReject`. The check uses the engine's
     * internal Promise slot, so an object that merely inherits from
     * `Promise.prototype` is reported as an ordinary return.
     *
     * @param {() => unknown} callback
     * @param {string} [message]
     * @throws {TypeError} for a bad argument or a Promise-returning callback.
     * @throws {AssertionError} when the callback throws.
     */
    function doesNotThrow(callback, message) {
        checkArity(arguments, "assert.doesNotThrow", 1, 2);
        checkMessage(message);
        checkCallback(callback);
        let returned;
        try {
            returned = callback();
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the function not to throw, but it threw ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotThrow" },
            );
        }
        if (isNativePromise(returned)) {
            settlePromise(returned);
            throw new makeTypeError(
                "assert.doesNotThrow expects a synchronous function; " +
                    "use assert.doesNotReject for a Promise-returning function",
            );
        }
        return returned;
    }

    /**
     * Resolve with the rejection reason once `callback` rejects; reject with an
     * `AssertionError` when it fulfils.
     *
     * A synchronous throw is treated as a rejection. `expected` has the same
     * meaning as in `throws`. The returned Promise rejects for invalid
     * arguments. The callback's return value must be a genuine native Promise
     * or a thenable. A thenable is assimilated by reading `then` once and
     * calling it once. A throwing `then` accessor or a throwing `then` call is
     * treated as the target's rejection and checked against `expected`; it
     * therefore normally resolves with that error when `expected` is omitted
     * or matches, and throws `AssertionError` only when the check fails. A
     * non-thenable return is an `AssertionError`.
     *
     * A non-thenable rejection reason is returned as the Promise value. Because
     * this is an async function, a thenable or Promise reason is assimilated by
     * native Promise resolution, so the returned Promise can instead fulfil
     * with that reason's value or reject with its rejection. The assertion has
     * already passed when the target rejected.
     *
     * @param {() => unknown} callback
     * @param {Function|RegExp|null} [expected]
     * @param {string} [message]
     * @returns {Promise<unknown>} the rejection reason, subject to native
     * Promise assimilation when the reason is a thenable.
     */
    async function rejects(callback, expected, message) {
        checkArity(arguments, "assert.rejects", 1, 3);
        checkMessage(message);
        checkCallback(callback);
        checkExpected(expected);
        let result;
        try {
            result = callback();
        } catch (error) {
            return checkThrown(error, expected, message, "rejects");
        }
        if (isNativePromise(result)) {
            const observation = observePromise(result);
            try {
                await observation.promise;
            } catch (error) {
                throw new AssertionError(
                    withMessage(
                        message,
                        `could not inspect the Promise: ${formatValue(error)}`,
                    ),
                    { actual: error, operator: "rejects" },
                );
            }
            if (observation.status === OBSERVE_REJECTED) {
                return checkThrown(observation.value, expected, message, "rejects");
            }
            if (observation.status !== OBSERVE_FULFILLED) {
                const detail =
                    observation.status === OBSERVE_FAILED
                        ? `: ${formatValue(observation.error)}`
                        : "";
                throw new AssertionError(
                    withMessage(message, `could not inspect the Promise${detail}`),
                    { actual: observation.error, operator: "rejects" },
                );
            }
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the Promise to reject, " +
                        `but it fulfilled with ${formatValue(observation.value)}`,
                ),
                { actual: observation.value, operator: "rejects" },
            );
        }
        let value;
        try {
            value = await assimilateThenable(result);
        } catch (error) {
            return checkThrown(error, expected, message, "rejects");
        }
        if (value === NOT_THENABLE) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function to return a Promise, " +
                        `but it returned ${formatValue(result)}`,
                ),
                { actual: result, operator: "rejects" },
            );
        }
        throw new AssertionError(
            withMessage(
                message,
                `expected the Promise to reject, but it fulfilled with ${formatValue(value)}`,
            ),
            { actual: value, operator: "rejects" },
        );
    }

    /**
     * Resolve with the fulfilment value once `callback` fulfils; reject with an
     * `AssertionError` when it throws or rejects.
     *
     * The callback's return value must be a genuine native Promise or a
     * thenable, assimilated exactly once as in `rejects`.
     *
     * @param {() => unknown} callback
     * @param {string} [message]
     * @returns {Promise<unknown>} the fulfilment value.
     */
    async function doesNotReject(callback, message) {
        checkArity(arguments, "assert.doesNotReject", 1, 2);
        checkMessage(message);
        checkCallback(callback);
        let result;
        try {
            result = callback();
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function not to throw or reject, " +
                        `but it threw ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotReject" },
            );
        }
        if (isNativePromise(result)) {
            const observation = observePromise(result);
            try {
                await observation.promise;
            } catch (error) {
                throw new AssertionError(
                    withMessage(
                        message,
                        `could not inspect the Promise: ${formatValue(error)}`,
                    ),
                    { actual: error, operator: "doesNotReject" },
                );
            }
            if (observation.status === OBSERVE_REJECTED) {
                throw new AssertionError(
                    withMessage(
                        message,
                        "expected the Promise not to reject, " +
                            `but it rejected with ${formatValue(observation.value)}`,
                    ),
                    { actual: observation.value, operator: "doesNotReject" },
                );
            }
            if (observation.status !== OBSERVE_FULFILLED) {
                const detail =
                    observation.status === OBSERVE_FAILED
                        ? `: ${formatValue(observation.error)}`
                        : "";
                throw new AssertionError(
                    withMessage(message, `could not inspect the Promise${detail}`),
                    { actual: observation.error, operator: "doesNotReject" },
                );
            }
            return observation.value;
        }
        let value;
        try {
            value = await assimilateThenable(result);
        } catch (error) {
            throw new AssertionError(
                withMessage(
                    message,
                    `expected the Promise not to reject, but it rejected with ${formatValue(error)}`,
                ),
                { actual: error, operator: "doesNotReject" },
            );
        }
        if (value === NOT_THENABLE) {
            throw new AssertionError(
                withMessage(
                    message,
                    "expected the function to return a Promise, " +
                        `but it returned ${formatValue(result)}`,
                ),
                { actual: result, operator: "doesNotReject" },
            );
        }
        return value;
    }

    // The module object is the callable `assert` itself. Its own properties are
    // the named assertions, and the module namespace re-exports the same
    // function objects. `ok` is the callable itself, as in Node's assert.
    defineField(assert, "AssertionError", AssertionError);
    defineField(assert, "ok", assert);
    defineField(assert, "equals", equals);
    defineField(assert, "notEquals", notEquals);
    defineField(assert, "strictEquals", strictEquals);
    defineField(assert, "notStrictEquals", notStrictEquals);
    defineField(assert, "fail", fail);
    defineField(assert, "throws", throws);
    defineField(assert, "doesNotThrow", doesNotThrow);
    defineField(assert, "rejects", rejects);
    defineField(assert, "doesNotReject", doesNotReject);
    freeze(assert);
    return assert;
})
