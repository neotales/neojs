// The engine-independent implementation of the `slices` built-in module.
//
// This file is spliced into the V8 realm shim only when `neo-slices` is
// enabled. It is not a module and has no imports or exports. It follows the
// shared `chars` declaration in `neo.js`, because `CharSlice` captures
// `chars.toUpper` and `chars.toLower`; the Cargo feature dependency makes that
// ordering mandatory.
//
// The implementation captures the intrinsics it uses at evaluation time. A
// disabled `neo-slices` build does not compile this file into the binary at all,
// so its source bytes are absent rather than merely unregistered.

    // `slices` creates live views over a JS Array or a genuine typed array.
    // A view addresses one fixed half-open range and does not copy for
    // construction, `slice`, `at`, `set`, `indexOf`, `includes`, or iteration.
    // `copy` and the character helpers `toString`/`toArray` are the explicit
    // copying operations.
    //
    // The range is fixed at construction: splicing, truncating, or extending
    // the target afterwards does not move or resize the view. Element reads and
    // writes always go to the current target, so a view of an Array keeps
    // aliasing the same absolute indices; removing one makes the read
    // `undefined`, and writing it follows ordinary Array semantics and can
    // extend the Array. A detached or shrunk typed array leaves reads
    // `undefined` and writes ignored; `copy` and the character conversions throw
    // RangeError when the fixed range no longer fits, instead of inventing
    // zeros. The check covers an empty range too: an empty view at offset 0
    // still fits a zero-length target, while an empty view at a nonzero offset
    // throws. Frozen Arrays make writes throw
    // TypeError. Accessor elements and Proxy traps run, because a live view is
    // not a snapshot and is not an isolation boundary.
    //
    // Target validation uses the engine's typed-array internal slots, not
    // `Symbol.toStringTag`, `Symbol.hasInstance`, or a `byteLength` getter. A
    // DataView, a lookalike object, and a Proxy of a typed array are rejected;
    // a Proxy of an Array is accepted and its traps run. Typed-array views share
    // the target's buffer, so views of overlapping ranges alias each other.
    // Element types and offsets are preserved, including BigInt element types
    // and `Uint8ClampedArray`. `copy` constructs a plain Array or the same
    // intrinsic typed-array type directly, so `Symbol.species` is never
    // consulted.
    //
    // `CharSlice` is the character workflow: a caller-owned `Uint32Array` of
    // Unicode code points, converted once from a JS string at the boundary and
    // transformed in place. JS strings are immutable, so no view can alias
    // string storage. `fromString` allocates one code point buffer and a view
    // over it (a shorter view over the same allocation when surrogate pairs
    // require trimming). `toString` allocates the result string and bounded
    // chunk temporaries, not a string per code point. `slice` allocates only a
    // view/state object and no character buffer. `toUpper`/`toLower` allocate no
    // new character buffer, but their simple case mapping may allocate a
    // temporary string per mapped code point; they avoid converting the whole
    // buffer to a string.
    //
    // This is plain ECMAScript with no host op. It adds no capability, is not a
    // sandbox, and is not a Rust, cross-realm, or cross-thread zero-copy bridge.
    const slices = (() => {
        "use strict";

        const isInteger = Number.isInteger;
        const makeTypeError = TypeError;
        const makeRangeError = RangeError;
        const arrayIsArray = Array.isArray;
        const getPrototypeOf = Object.getPrototypeOf;
        const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
        const objectCreate = Object.create;
        const freeze = Object.freeze;
        const makeMap = Map;
        const mapGet = Function.prototype.call.bind(Map.prototype.get);
        const mapSet = Function.prototype.call.bind(Map.prototype.set);
        const makeUint8Array = Uint8Array;
        const makeUint32Array = Uint32Array;
        const fromCodePoint = String.fromCodePoint;
        const codePointAt = Function.prototype.call.bind(String.prototype.codePointAt);
        const mathMin = Math.min;
        const reflectApply = Reflect.apply;
        const arrayJoin = Function.prototype.call.bind(Array.prototype.join);
        const symbolIterator = Symbol.iterator;
        const symbolToStringTag = Symbol.toStringTag;

        // The getters live on `%TypedArray%.prototype`. Reading them from the
        // intrinsic prototype avoids a replaced `Symbol.toStringTag` or a
        // shadowed `length` on the target. `typedArrayTag` returns `undefined`
        // for anything that is not a genuine typed array and for a Proxy of one.
        const typedArrayPrototype = getPrototypeOf(makeUint8Array.prototype);
        const typedArrayLength = getOwnPropertyDescriptor(typedArrayPrototype, "length").get;
        const typedArrayBuffer = getOwnPropertyDescriptor(typedArrayPrototype, "buffer").get;
        const typedArrayTag = getOwnPropertyDescriptor(typedArrayPrototype, symbolToStringTag).get;

        // Bind the getters now. Calling `getter.call(value)` later would resolve
        // `Function.prototype.call` dynamically; user code can replace it to
        // receive the getter and the private typed-array argument. A bound
        // wrapper invokes the captured getter without that lookup.
        const typedArrayLengthOf = Function.prototype.call.bind(typedArrayLength);
        const typedArrayBufferOf = Function.prototype.call.bind(typedArrayBuffer);
        const typedArrayTagOf = Function.prototype.call.bind(typedArrayTag);

        /** The intrinsic typed-array name, or `undefined` for a non-typed-array. */
        function typedArrayName(value) {
            try {
                return typedArrayTagOf(value);
            } catch {
                return undefined;
            }
        }

        /** Intrinsic constructors for `copy`, keyed by the engine's type name. */
        const typedArrayConstructors = new makeMap();
        function rememberTypedArray(name, constructor) {
            if (typeof constructor === "function") {
                mapSet(typedArrayConstructors, name, constructor);
            }
        }
        rememberTypedArray("Int8Array", Int8Array);
        rememberTypedArray("Uint8Array", Uint8Array);
        rememberTypedArray("Uint8ClampedArray", Uint8ClampedArray);
        rememberTypedArray("Int16Array", Int16Array);
        rememberTypedArray("Uint16Array", Uint16Array);
        rememberTypedArray("Int32Array", Int32Array);
        rememberTypedArray("Uint32Array", Uint32Array);
        rememberTypedArray("Float32Array", Float32Array);
        rememberTypedArray("Float64Array", Float64Array);
        rememberTypedArray("BigInt64Array", BigInt64Array);
        rememberTypedArray("BigUint64Array", BigUint64Array);
        if (typeof Float16Array === "function") {
            rememberTypedArray("Float16Array", Float16Array);
        }

        /** `"Array"` or the typed-array name, or throw `TypeError`. */
        function targetKind(target) {
            let isArray = false;
            try {
                isArray = arrayIsArray(target);
            } catch {
                isArray = false;
            }
            if (isArray) return "Array";
            const name = typedArrayName(target);
            if (name !== undefined) return name;
            throw makeTypeError("slices target must be an Array or a typed array");
        }

        /** The target's current element count. */
        function targetLength(target, kind) {
            if (kind === "Array") {
                const length = target.length;
                if (
                    typeof length !== "number" ||
                    !isInteger(length) ||
                    length < 0 ||
                    length > 0xffffffff
                ) {
                    throw makeTypeError("slices target has an invalid length");
                }
                return length;
            }
            return typedArrayLengthOf(target);
        }

        /**
         * Resolve a half-open `[start, end)` range against `length`.
         * `undefined` means the default (zero or `length`). Bounds are not
         * clamped and negative indices are not supported.
         */
        function resolveRange(length, start, end) {
            if (start === undefined) start = 0;
            if (end === undefined) end = length;
            if (typeof start !== "number") {
                throw makeTypeError("slice start must be a number");
            }
            if (typeof end !== "number") {
                throw makeTypeError("slice end must be a number");
            }
            if (!isInteger(start)) throw makeRangeError("slice start must be an integer");
            if (!isInteger(end)) throw makeRangeError("slice end must be an integer");
            if (start < 0 || end < 0) {
                throw makeRangeError("slice bounds must not be negative");
            }
            if (start > end) {
                throw makeRangeError("slice start must not be greater than end");
            }
            if (end > length) {
                throw makeRangeError("slice end must not exceed the target length");
            }
            return { start, end, length: end - start };
        }

        /** Require an in-view integer index. */
        function requireIndex(index, length) {
            if (typeof index !== "number") {
                throw makeTypeError("slice index must be a number");
            }
            if (!isInteger(index)) throw makeRangeError("slice index must be an integer");
            if (index < 0 || index >= length) {
                throw makeRangeError("slice index is out of range");
            }
        }

        /**
         * Fixed view state. The public constructors validate; internal nested
         * views copy the already-validated range without re-reading a target
         * that may have shrunk since the parent view was created.
         */
        class SliceState {
            constructor(target, start, end) {
                const kind = targetKind(target);
                const length = targetLength(target, kind);
                const range = resolveRange(length, start, end);
                this.target = target;
                this.kind = kind;
                this.start = range.start;
                this.length = range.length;
            }
        }

        function stateFrom(state, start, length) {
            const next = objectCreate(SliceState.prototype);
            next.target = state.target;
            next.kind = state.kind;
            next.start = start;
            next.length = length;
            return next;
        }

        function viewAt(state, index) {
            requireIndex(index, state.length);
            return state.target[state.start + index];
        }

        function viewSet(state, index, value) {
            requireIndex(index, state.length);
            state.target[state.start + index] = value;
        }

        function viewIndexOf(state, value) {
            for (let i = 0; i < state.length; i++) {
                if (state.target[state.start + i] === value) return i;
            }
            return -1;
        }

        function viewIncludes(state, value) {
            for (let i = 0; i < state.length; i++) {
                const item = state.target[state.start + i];
                // SameValueZero, like Array.prototype.includes: NaN is found.
                if (item === value || (item !== item && value !== value)) return true;
            }
            return false;
        }

        function viewIterator(state) {
            let index = 0;
            return {
                next() {
                    if (index < state.length) {
                        const value = state.target[state.start + index];
                        index += 1;
                        return { done: false, value };
                    }
                    return { done: true, value: undefined };
                },
            };
        }

        function arrayCopy(state) {
            const out = [];
            for (let i = 0; i < state.length; i++) {
                out[i] = state.target[state.start + i];
            }
            return out;
        }

        function typedArrayCopy(state) {
            const constructor = mapGet(typedArrayConstructors, state.kind);
            if (constructor === undefined) {
                throw makeTypeError(
                    `slices cannot copy an unsupported typed array: ${state.kind}`,
                );
            }
            // Check the fixed range as a whole, including an empty range. An
            // empty view at offset 0 still fits a detached (zero-length)
            // target; an empty view at a nonzero offset no longer does.
            if (state.start + state.length > typedArrayLengthOf(state.target)) {
                throw makeRangeError("slices view no longer fits its typed array");
            }
            const out = new constructor(state.length);
            for (let i = 0; i < state.length; i++) {
                out[i] = state.target[state.start + i];
            }
            return out;
        }

        function viewCopy(state) {
            return state.kind === "Array" ? arrayCopy(state) : typedArrayCopy(state);
        }

        // The token lets `slice` create an already-validated nested view. It is
        // private to this block and cannot be produced by user code.
        const VIEW_STATE = Symbol("sliceViewState");

        /**
         * A mutable live view over an Array or typed array.
         *
         * Construction, `slice`, `at`, `set`, `indexOf`, `includes`, and
         * iteration do not copy. `copy` is the only snapshot operation.
         */
        class Slice {
            #state;

            /**
             * Create a mutable view of `target` over `[start, end)`.
             *
             * @param {Array|TypedArray} target the live target
             * @param {number} [start=0] first element index, inclusive
             * @param {number} [end=target.length] last element index, exclusive
             * @throws {TypeError} when the target is not an Array or genuine
             * typed array, or a bound is not a number.
             * @throws {RangeError} when a bound is not an integer, is negative,
             * is reversed, or exceeds the target length.
             */
            constructor(target, start = 0, end = undefined) {
                if (target === VIEW_STATE) {
                    this.#state = start;
                    return;
                }
                this.#state = new SliceState(target, start, end);
            }

            /** Number of elements in the view; fixed at construction. */
            get length() {
                return this.#state.length;
            }

            /** Whether the view is empty. */
            get isEmpty() {
                return this.#state.length === 0;
            }

            /**
             * Read the element at view index `index`.
             *
             * @throws {TypeError} when `index` is not a number.
             * @throws {RangeError} when `index` is not an integer or is outside
             * the view.
             */
            at(index) {
                return viewAt(this.#state, index);
            }

            /**
             * Write the element at view index `index` through to the target.
             *
             * @returns {this} the view, for chaining
             * @throws {TypeError} when `index` is not a number; also whatever the
             * target throws, such as `TypeError` for a frozen Array element or a
             * BigInt element assigned a number.
             * @throws {RangeError} when `index` is not an integer or is outside
             * the view.
             */
            set(index, value) {
                viewSet(this.#state, index, value);
                return this;
            }

            /**
             * Create a nested mutable view. `start` and `end` are relative to
             * this view, and the result aliases the same target range. The
             * nested view keeps its bounds even if the target shrinks later.
             *
             * @throws {TypeError} when a bound is not a number.
             * @throws {RangeError} when a bound is not an integer, is negative,
             * is reversed, or exceeds this view's length.
             */
            slice(start = 0, end = undefined) {
                const range = resolveRange(this.#state.length, start, end);
                return new Slice(
                    VIEW_STATE,
                    stateFrom(this.#state, this.#state.start + range.start, range.length),
                );
            }

            /**
             * Copy the current range into a new plain Array (for an Array
             * target) or a new intrinsic typed array of the same type. The copy
             * never aliases and never consults `Symbol.species`. Array holes
             * copied as properties become `undefined` own elements.
             *
             * @throws {RangeError} when a typed-array target no longer covers
             * the view (for example after its buffer was detached or shrunk).
             */
            copy() {
                return viewCopy(this.#state);
            }

            /** First view index whose element is strictly equal to `value`, or `-1`. */
            indexOf(value) {
                return viewIndexOf(this.#state, value);
            }

            /** Whether any element is `value` under SameValueZero (`NaN` matches). */
            includes(value) {
                return viewIncludes(this.#state, value);
            }

            /** Iterate the current elements in view order. */
            [symbolIterator]() {
                return viewIterator(this.#state);
            }
        }

        /**
         * A read-only live view over an Array or typed array. It has the same
         * reads and snapshot behaviour as `Slice`, without `set`. Reads still
         * observe writes made through another view or the target.
         */
        class ReadOnlySlice {
            #state;

            /**
             * Create a read-only view of `target` over `[start, end)`.
             *
             * @throws {TypeError} when the target is not an Array or genuine
             * typed array, or a bound is not a number.
             * @throws {RangeError} when a bound is not an integer, is negative,
             * is reversed, or exceeds the target length.
             */
            constructor(target, start = 0, end = undefined) {
                if (target === VIEW_STATE) {
                    this.#state = start;
                    return;
                }
                this.#state = new SliceState(target, start, end);
            }

            /** Number of elements in the view; fixed at construction. */
            get length() {
                return this.#state.length;
            }

            /** Whether the view is empty. */
            get isEmpty() {
                return this.#state.length === 0;
            }

            /** Read the element at view index `index`. */
            at(index) {
                return viewAt(this.#state, index);
            }

            /** Create a nested read-only view over the same target range. */
            slice(start = 0, end = undefined) {
                const range = resolveRange(this.#state.length, start, end);
                return new ReadOnlySlice(
                    VIEW_STATE,
                    stateFrom(this.#state, this.#state.start + range.start, range.length),
                );
            }

            /** Copy the current range; see `Slice.prototype.copy`. */
            copy() {
                return viewCopy(this.#state);
            }

            /** First view index whose element is strictly equal to `value`, or `-1`. */
            indexOf(value) {
                return viewIndexOf(this.#state, value);
            }

            /** Whether any element is `value` under SameValueZero (`NaN` matches). */
            includes(value) {
                return viewIncludes(this.#state, value);
            }

            /** Iterate the current elements in view order. */
            [symbolIterator]() {
                return viewIterator(this.#state);
            }
        }

        /** A Unicode scalar value: 0 through 0x10FFFF, excluding surrogates. */
        function isScalar(value) {
            return (
                isInteger(value) &&
                value >= 0 &&
                value <= 0x10ffff &&
                (value < 0xd800 || value > 0xdfff)
            );
        }

        /** Fixed state for a `CharSlice` over a caller-owned `Uint32Array`. */
        class CharState {
            constructor(buffer, start, end) {
                if (typedArrayName(buffer) !== "Uint32Array") {
                    throw makeTypeError("CharSlice buffer must be a Uint32Array");
                }
                const length = typedArrayLengthOf(buffer);
                const range = resolveRange(length, start, end);
                this.buffer = buffer;
                this.start = range.start;
                this.length = range.length;
            }
        }

        function charStateFrom(state, start, length) {
            const next = objectCreate(CharState.prototype);
            next.buffer = state.buffer;
            next.start = start;
            next.length = length;
            return next;
        }

        function charRequireBounds(state) {
            // Match `typedArrayCopy`: validate the fixed range even when it is
            // empty. An empty view at offset 0 still fits a detached buffer.
            if (state.start + state.length > typedArrayLengthOf(state.buffer)) {
                throw makeRangeError("character slice no longer fits its buffer");
            }
        }

        function charAt(state, index) {
            requireIndex(index, state.length);
            return state.buffer[state.start + index];
        }

        function charSet(state, index, value) {
            requireIndex(index, state.length);
            if (typeof value !== "number") {
                throw makeTypeError("character slice value must be a number");
            }
            if (!isInteger(value)) {
                throw makeRangeError("character slice value must be an integer");
            }
            if (value < 0 || value > 0x10ffff) {
                throw makeRangeError("character slice value is outside the Unicode range");
            }
            state.buffer[state.start + index] = value;
        }

        function charToString(state) {
            charRequireBounds(state);
            const pieces = [];
            for (let offset = 0; offset < state.length; offset += 4096) {
                const size = mathMin(4096, state.length - offset);
                const chunk = [];
                for (let i = 0; i < size; i++) {
                    const value = state.buffer[state.start + offset + i];
                    if (value > 0x10ffff) {
                        throw makeRangeError(
                            "character slice contains a value outside the Unicode range",
                        );
                    }
                    chunk[i] = value;
                }
                pieces[pieces.length] = reflectApply(fromCodePoint, null, chunk);
            }
            return arrayJoin(pieces, "");
        }

        function charToArray(state) {
            charRequireBounds(state);
            const out = new makeUint32Array(state.length);
            for (let i = 0; i < state.length; i++) {
                out[i] = state.buffer[state.start + i];
            }
            return out;
        }

        function charApplyCase(state, mapper) {
            charRequireBounds(state);
            for (let i = 0; i < state.length; i++) {
                const index = state.start + i;
                const value = state.buffer[index];
                // `chars.toUpper`/`toLower` reject surrogates and invalid
                // values; leave those unchanged so a buffer that came from a
                // JS string, including a lone surrogate, roundtrips.
                if (isScalar(value)) state.buffer[index] = mapper(value);
            }
        }

        function charIterator(state) {
            let index = 0;
            return {
                next() {
                    if (index < state.length) {
                        const value = state.buffer[state.start + index];
                        index += 1;
                        return { done: false, value };
                    }
                    return { done: true, value: undefined };
                },
            };
        }

        /** One `Uint32Array` of code points, with surrogate pairs combined. */
        function codePointsFromString(text) {
            if (typeof text !== "string") {
                throw makeTypeError("text must be a string");
            }
            const length = text.length;
            const buffer = new makeUint32Array(length);
            let count = 0;
            for (let i = 0; i < length; ) {
                const value = codePointAt(text, i);
                buffer[count] = value;
                count += 1;
                i += value > 0xffff ? 2 : 1;
            }
            if (count === length) return buffer;
            // A new view over the same allocation, not `subarray`: `subarray`
            // consults `Symbol.species`. Read the backing buffer through the
            // captured intrinsic getter, not `buffer.buffer`, whose prototype
            // getter later code can replace to capture this private allocation.
            const backing = typedArrayBufferOf(buffer);
            return new makeUint32Array(backing, 0, count);
        }

        const CHAR_STATE = Symbol("charSliceState");
        const charToUpper = chars.toUpper;
        const charToLower = chars.toLower;

        /**
         * A mutable live view over Unicode code points in a caller-owned
         * `Uint32Array`.
         *
         * `fromString` allocates one code point buffer and a view (plus a
         * shorter view over the same allocation for astral text). `toString`
         * allocates the result string and bounded chunk temporaries, not a
         * string per code point. `slice` allocates only a view/state object and
         * no character buffer. `toUpper`/`toLower` allocate no new character
         * buffer, but the simple case mapping may allocate a temporary string
         * per mapped code point. JS strings are immutable, so no operation
         * aliases string storage.
         */
        class CharSlice {
            #state;

            /**
             * Create a mutable character view over `buffer[start..end)`.
             *
             * @param {Uint32Array} buffer a caller-owned code point buffer
             * @throws {TypeError} when `buffer` is not a genuine `Uint32Array`
             * or a bound is not a number.
             * @throws {RangeError} when a bound is not an integer, is negative,
             * is reversed, or exceeds the buffer length.
             */
            constructor(buffer, start = 0, end = undefined) {
                if (buffer === CHAR_STATE) {
                    this.#state = start;
                    return;
                }
                this.#state = new CharState(buffer, start, end);
            }

            /**
             * Convert a string to a mutable character view. Allocates one new
             * code point buffer and a view over it; surrogate pairs become one
             * code point, while a lone surrogate stays one code unit. Astral
             * text uses a shorter view over the same allocation, not a copy.
             */
            static fromString(text) {
                return new CharSlice(codePointsFromString(text));
            }

            /** Number of code points in the view; fixed at construction. */
            get length() {
                return this.#state.length;
            }

            /** Whether the view is empty. */
            get isEmpty() {
                return this.#state.length === 0;
            }

            /** Read the code point at view index `index`. */
            at(index) {
                return charAt(this.#state, index);
            }

            /**
             * Write a code point through to the buffer.
             *
             * @returns {this} the view, for chaining
             * @throws {TypeError} when the index or value is not a number.
             * @throws {RangeError} when the index is not an integer or is
             * outside the view, or the value is not a Unicode code point.
             */
            set(index, value) {
                charSet(this.#state, index, value);
                return this;
            }

            /** Create a nested mutable view over the same buffer range. */
            slice(start = 0, end = undefined) {
                const range = resolveRange(this.#state.length, start, end);
                return new CharSlice(
                    CHAR_STATE,
                    charStateFrom(this.#state, this.#state.start + range.start, range.length),
                );
            }

            /**
             * Apply `chars.toUpper` in place to every scalar value. Expanding
             * mappings such as `ß` to `SS` are left unchanged, as are lone
             * surrogates and values outside the Unicode range. This allocates
             * no new character buffer; the simple case mapping may allocate a
             * temporary string per mapped code point.
             *
             * @returns {this} the view, for chaining
             */
            toUpper() {
                charApplyCase(this.#state, charToUpper);
                return this;
            }

            /**
             * Apply `chars.toLower` in place to every scalar value; see
             * `toUpper`.
             *
             * @returns {this} the view, for chaining
             */
            toLower() {
                charApplyCase(this.#state, charToLower);
                return this;
            }

            /**
             * Build a new string. Allocates the result and bounded chunk
             * temporaries, not a string per code point.
             */
            toString() {
                return charToString(this.#state);
            }

            /** Copy the range into a new `Uint32Array` that does not alias. */
            toArray() {
                return charToArray(this.#state);
            }

            /** Iterate code points in view order. */
            [symbolIterator]() {
                return charIterator(this.#state);
            }
        }

        /**
         * A read-only live view over Unicode code points in a caller-owned
         * `Uint32Array`. It has no in-place transforms; make a mutable
         * `CharSlice` over the same buffer to transform it.
         */
        class ReadOnlyCharSlice {
            #state;

            /**
             * Create a read-only character view over `buffer[start..end)`.
             *
             * @throws {TypeError} when `buffer` is not a genuine `Uint32Array`
             * or a bound is not a number.
             * @throws {RangeError} when a bound is not an integer, is negative,
             * is reversed, or exceeds the buffer length.
             */
            constructor(buffer, start = 0, end = undefined) {
                if (buffer === CHAR_STATE) {
                    this.#state = start;
                    return;
                }
                this.#state = new CharState(buffer, start, end);
            }

            /**
             * Convert a string to a read-only character view; allocates one
             * code point buffer and a view over it.
             */
            static fromString(text) {
                return new ReadOnlyCharSlice(codePointsFromString(text));
            }

            /** Number of code points in the view; fixed at construction. */
            get length() {
                return this.#state.length;
            }

            /** Whether the view is empty. */
            get isEmpty() {
                return this.#state.length === 0;
            }

            /** Read the code point at view index `index`. */
            at(index) {
                return charAt(this.#state, index);
            }

            /** Create a nested read-only view over the same buffer range. */
            slice(start = 0, end = undefined) {
                const range = resolveRange(this.#state.length, start, end);
                return new ReadOnlyCharSlice(
                    CHAR_STATE,
                    charStateFrom(this.#state, this.#state.start + range.start, range.length),
                );
            }

            /**
             * Build a new string. Allocates the result and bounded chunk
             * temporaries, not a string per code point.
             */
            toString() {
                return charToString(this.#state);
            }

            /** Copy the range into a new `Uint32Array` that does not alias. */
            toArray() {
                return charToArray(this.#state);
            }

            /** Iterate code points in view order. */
            [symbolIterator]() {
                return charIterator(this.#state);
            }
        }

        /**
         * Create a mutable live view over an Array or typed array.
         *
         * @param {Array|TypedArray} target the live target
         * @param {number} [start=0] first element index, inclusive
         * @param {number} [end=target.length] last element index, exclusive
         */
        function slice(target, start = 0, end = undefined) {
            return new Slice(target, start, end);
        }

        /** Create a read-only live view over an Array or typed array. */
        function readonlySlice(target, start = 0, end = undefined) {
            return new ReadOnlySlice(target, start, end);
        }

        /** Create a mutable live view over a `Uint32Array` of code points. */
        function charSlice(buffer, start = 0, end = undefined) {
            return new CharSlice(buffer, start, end);
        }

        /** Create a read-only live view over a `Uint32Array` of code points. */
        function readonlyCharSlice(buffer, start = 0, end = undefined) {
            return new ReadOnlyCharSlice(buffer, start, end);
        }

        return freeze({
            CharSlice,
            ReadOnlyCharSlice,
            ReadOnlySlice,
            Slice,
            charSlice,
            readonlyCharSlice,
            readonlySlice,
            slice,
        });
    })();
