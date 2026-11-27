(host) => {
    "use strict";

    const isV8 = Boolean(host.isV8);
    const MAX_CHILD_ARGS = 4096;
    const MAX_CHILD_ENV = 4096;
    const MAX_MAX_BUFFER = 64 * 1024 * 1024;

    const objectCreate = Object.create;
    const objectKeys = Object.keys;
    const objectDefineProperty = Object.defineProperty;
    const getPrototypeOf = Object.getPrototypeOf;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const arrayIsArray = Array.isArray;
    const numberIsInteger = Number.isInteger;
    const numberIsFinite = Number.isFinite;
    const mathTrunc = Math.trunc;
    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const makeUint8Array = Uint8Array;
    const typedArrayProto = getPrototypeOf(makeUint8Array.prototype);
    const dataViewProto = DataView.prototype;
    const arrayBufferProto = ArrayBuffer.prototype;
    const callBind = Function.prototype.call.bind(Function.prototype.bind);
    const fnCallBind = (fn) => callBind(Function.prototype.call, fn);
    const hasOwn = fnCallBind(Object.prototype.hasOwnProperty);

    function ownOpt(obj, key) {
        return hasOwn(obj, key) ? obj[key] : undefined;
    }

    const taBufferOf = fnCallBind(getOwnPropertyDescriptor(typedArrayProto, "buffer").get);
    const taByteOffsetOf = fnCallBind(getOwnPropertyDescriptor(typedArrayProto, "byteOffset").get);
    const taByteLengthOf = fnCallBind(getOwnPropertyDescriptor(typedArrayProto, "byteLength").get);
    const taTagOf = fnCallBind(getOwnPropertyDescriptor(typedArrayProto, Symbol.toStringTag).get);
    const typedArraySet = fnCallBind(typedArrayProto.set);
    const typedArraySubarray = fnCallBind(typedArrayProto.subarray);

    const dvBufferOf = fnCallBind(getOwnPropertyDescriptor(dataViewProto, "buffer").get);
    const dvByteOffsetOf = fnCallBind(getOwnPropertyDescriptor(dataViewProto, "byteOffset").get);
    const dvByteLengthOf = fnCallBind(getOwnPropertyDescriptor(dataViewProto, "byteLength").get);

    const abByteLengthOf = fnCallBind(getOwnPropertyDescriptor(arrayBufferProto, "byteLength").get);
    const sabByteLengthOf = typeof SharedArrayBuffer === "function"
        ? fnCallBind(getOwnPropertyDescriptor(SharedArrayBuffer.prototype, "byteLength").get)
        : null;

    function defineDataProp(obj, name, value) {
        const desc = objectCreate(null);
        desc.value = value;
        desc.writable = true;
        desc.enumerable = true;
        desc.configurable = true;
        objectDefineProperty(obj, name, desc);
    }

    function isRealArrayBuffer(val) {
        if (val === null || typeof val !== "object") return false;
        try {
            abByteLengthOf(val);
            return true;
        } catch {
            if (sabByteLengthOf !== null) {
                try {
                    sabByteLengthOf(val);
                    return true;
                } catch {
                    return false;
                }
            }
            return false;
        }
    }

    function readViewSlice(value) {
        if (value === null || typeof value !== "object") return null;
        try {
            if (taTagOf(value) !== undefined) {
                return new makeUint8Array(taBufferOf(value), taByteOffsetOf(value), taByteLengthOf(value));
            }
        } catch {
            // Fall through to DataView check.
        }
        try {
            const buf = dvBufferOf(value);
            return new makeUint8Array(buf, dvByteOffsetOf(value), dvByteLengthOf(value));
        } catch {
            return null;
        }
    }

    function assertString(value, name) {
        if (typeof value !== "string") {
            throw new makeTypeError(`${name} must be a string`);
        }
        return value;
    }

    function normalizeEncoding(enc, fallback = "utf8") {
        if (enc === undefined || enc === null) {
            return fallback;
        }
        if (typeof enc !== "string") {
            throw new makeTypeError("encoding must be a string");
        }
        const lower = enc.toLowerCase();
        switch (lower) {
            case "utf8":
            case "utf-8":
                return "utf8";
            case "hex":
                return "hex";
            case "base64":
                return "base64";
            case "base64url":
                return "base64url";
            case "latin1":
            case "binary":
                return "latin1";
            case "ascii":
                return "ascii";
            default: {
                const err = new makeTypeError(`Unknown encoding: ${enc}`);
                defineDataProp(err, "code", "ERR_UNKNOWN_ENCODING");
                throw err;
            }
        }
    }

    function isKnownEncoding(enc) {
        if (typeof enc !== "string") return false;
        switch (enc.toLowerCase()) {
            case "utf8":
            case "utf-8":
            case "hex":
            case "base64":
            case "base64url":
            case "latin1":
            case "binary":
            case "ascii":
                return true;
            default:
                return false;
        }
    }

    function toBytes(input, encoding) {
        if (typeof input === "string") {
            return host.bufferDecode(input, normalizeEncoding(encoding, "utf8"));
        }
        const view = readViewSlice(input);
        if (view !== null) {
            return view;
        }
        if (isRealArrayBuffer(input)) {
            return new makeUint8Array(input);
        }
        throw new makeTypeError("data must be a string, TypedArray, DataView, or ArrayBuffer");
    }

    // -------------------------------------------------------------------------
    // node:buffer
    // -------------------------------------------------------------------------
    class Buffer extends Uint8Array {
        static from(value, encodingOrOffset, length) {
            if (typeof value === "string") {
                const enc = normalizeEncoding(encodingOrOffset, "utf8");
                const bytes = host.bufferDecode(value, enc);
                return new Buffer(taBufferOf(bytes), taByteOffsetOf(bytes), taByteLengthOf(bytes));
            }
            if (isRealArrayBuffer(value)) {
                const offset = encodingOrOffset ?? 0;
                if (length === undefined) {
                    return new Buffer(value, offset);
                }
                return new Buffer(value, offset, length);
            }
            const view = readViewSlice(value);
            if (view !== null) {
                const viewLen = taByteLengthOf(view);
                const copy = new makeUint8Array(viewLen);
                typedArraySet(copy, view);
                return new Buffer(taBufferOf(copy));
            }
            if (arrayIsArray(value) || (value !== null && typeof value === "object" && typeof value[Symbol.iterator] === "function")) {
                const arr = makeUint8Array.from(value);
                return new Buffer(taBufferOf(arr));
            }
            throw new makeTypeError("The first argument must be of type string, Buffer, ArrayBuffer, Array, or Array-like Object");
        }

        static alloc(size, fill, encoding) {
            if (!Number.isInteger(size) || size < 0) {
                throw new RangeError("Buffer size must be a non-negative integer");
            }
            const buf = new Buffer(size);
            if (fill !== undefined && size > 0) {
                if (typeof fill === "number") {
                    buf.fill(fill & 0xff);
                } else {
                    const fillBytes = typeof fill === "string"
                        ? host.bufferDecode(fill, normalizeEncoding(encoding, "utf8"))
                        : Buffer.from(fill);
                    if (fillBytes.length > 0) {
                        for (let offset = 0; offset < size; offset += fillBytes.length) {
                            const remaining = size - offset;
                            buf.set(
                                remaining >= fillBytes.length ? fillBytes : fillBytes.subarray(0, remaining),
                                offset,
                            );
                        }
                    }
                }
            }
            return buf;
        }

        static allocUnsafe(size) {
            if (!Number.isInteger(size) || size < 0) {
                throw new RangeError("Buffer size must be a non-negative integer");
            }
            return new Buffer(size);
        }

        static concat(list, totalLength) {
            if (!arrayIsArray(list)) {
                throw new makeTypeError("The \"list\" argument must be an instance of Array");
            }
            let total = 0;
            for (let i = 0; i < list.length; i++) {
                const item = list[i];
                let isU8 = false;
                try {
                    isU8 = taTagOf(item) === "Uint8Array";
                } catch {
                    isU8 = false;
                }
                if (!isU8) {
                    throw new makeTypeError("The \"list\" argument elements must be instances of Buffer or Uint8Array");
                }
                total += taByteLengthOf(item);
            }
            if (totalLength !== undefined) {
                if (!numberIsInteger(totalLength) || totalLength < 0) {
                    throw new makeRangeError("totalLength must be a non-negative integer");
                }
                total = totalLength;
            }
            const out = new Buffer(total);
            let offset = 0;
            for (let i = 0; i < list.length && offset < total; i++) {
                const item = list[i];
                const itemLen = taByteLengthOf(item);
                const remaining = total - offset;
                if (itemLen <= remaining) {
                    typedArraySet(out, item, offset);
                    offset += itemLen;
                } else {
                    typedArraySet(out, typedArraySubarray(item, 0, remaining), offset);
                    offset = total;
                }
            }
            return out;
        }

        static isBuffer(val) {
            return val instanceof Buffer;
        }

        static isEncoding(encoding) {
            return isKnownEncoding(encoding);
        }

        static byteLength(input, encoding) {
            if (typeof input === "string") {
                return host.bufferByteLength(input, normalizeEncoding(encoding, "utf8"));
            }
            if (ArrayBuffer.isView(input) || input instanceof ArrayBuffer || (typeof SharedArrayBuffer === "function" && input instanceof SharedArrayBuffer)) {
                return input.byteLength;
            }
            throw new TypeError("The \"string\" argument must be of type string or an instance of Buffer or ArrayBuffer");
        }

        toString(encoding = "utf8", start = 0, end = this.length) {
            const len = this.length;
            let s = Number(start);
            if (Number.isNaN(s)) s = 0;
            if (s < 0) s = Math.max(len + s, 0);
            if (s > len) s = len;

            let e = end === undefined ? len : Number(end);
            if (Number.isNaN(e)) e = 0;
            if (e < 0) e = Math.max(len + e, 0);
            if (e > len) e = len;

            const enc = normalizeEncoding(encoding, "utf8");
            if (s >= e) return "";
            return host.bufferEncode(this.subarray(s, e), enc);
        }

        toJSON() {
            return { type: "Buffer", data: Array.from(this) };
        }

        equals(other) {
            if (!(other instanceof Uint8Array)) {
                throw new TypeError("The \"otherBuffer\" argument must be an instance of Buffer or Uint8Array");
            }
            if (this.length !== other.length) return false;
            for (let i = 0; i < this.length; i++) {
                if (this[i] !== other[i]) return false;
            }
            return true;
        }
    }

    function toBuffer(u8) {
        if (u8 instanceof Buffer) return u8;
        return new Buffer(u8.buffer, u8.byteOffset, u8.byteLength);
    }

    const bufferMod = Object.freeze({ Buffer });

    // -------------------------------------------------------------------------
    // node:url
    // -------------------------------------------------------------------------
    const URL_INTERNAL = Symbol("urlInternal");
    const PARAMS_OWNER = Symbol("paramsOwner");

    function encodeFormComponent(str) {
        return encodeURIComponent(str)
            .replace(/%20/g, "+")
            .replace(/[!'()~*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
    }

    function decodeFormComponent(str) {
        return decodeURIComponent(str.replace(/\+/g, " "));
    }

    function parseQueryPairs(query) {
        const pairs = [];
        if (!query) return pairs;
        const raw = query.startsWith("?") ? query.slice(1) : query;
        if (!raw) return pairs;
        const parts = raw.split("&");
        for (const part of parts) {
            if (part === "") continue;
            const eq = part.indexOf("=");
            if (eq === -1) {
                pairs.push([decodeFormComponent(part), ""]);
            } else {
                pairs.push([
                    decodeFormComponent(part.slice(0, eq)),
                    decodeFormComponent(part.slice(eq + 1)),
                ]);
            }
        }
        return pairs;
    }

    class URLSearchParams {
        constructor(init = "") {
            this._entries = [];
            this[PARAMS_OWNER] = null;
            if (init === undefined || init === null || init === "") {
                return;
            }
            if (typeof init === "string") {
                this._entries = parseQueryPairs(init);
            } else if (init instanceof URLSearchParams) {
                for (const [k, v] of init._entries) {
                    this._entries.push([k, v]);
                }
            } else if (Array.isArray(init)) {
                for (const pair of init) {
                    if (!Array.isArray(pair) || pair.length !== 2) {
                        throw new TypeError("URLSearchParams sequence initializer elements must be [name, value] pairs");
                    }
                    this._entries.push([String(pair[0]), String(pair[1])]);
                }
            } else if (typeof init === "object") {
                if (typeof init[Symbol.iterator] === "function") {
                    for (const pair of init) {
                        if (!pair || typeof pair[Symbol.iterator] !== "function") {
                            throw new TypeError("URLSearchParams iterable elements must be [name, value] pairs");
                        }
                        const arr = Array.from(pair);
                        if (arr.length !== 2) {
                            throw new TypeError("URLSearchParams iterable elements must have length 2");
                        }
                        this._entries.push([String(arr[0]), String(arr[1])]);
                    }
                } else {
                    for (const key of Object.keys(init)) {
                        this._entries.push([String(key), String(init[key])]);
                    }
                }
            } else {
                this._entries = parseQueryPairs(String(init));
            }
        }

        _notify() {
            const owner = this[PARAMS_OWNER];
            if (owner) {
                owner._syncFromParams();
            }
        }

        get size() {
            return this._entries.length;
        }

        append(name, value) {
            this._entries.push([String(name), String(value)]);
            this._notify();
        }

        delete(name, value) {
            const key = String(name);
            if (value === undefined) {
                this._entries = this._entries.filter(([k]) => k !== key);
            } else {
                const val = String(value);
                this._entries = this._entries.filter(([k, v]) => !(k === key && v === val));
            }
            this._notify();
        }

        get(name) {
            const key = String(name);
            for (const [k, v] of this._entries) {
                if (k === key) return v;
            }
            return null;
        }

        getAll(name) {
            const key = String(name);
            const out = [];
            for (const [k, v] of this._entries) {
                if (k === key) out.push(v);
            }
            return out;
        }

        has(name, value) {
            const key = String(name);
            if (value === undefined) {
                return this._entries.some(([k]) => k === key);
            }
            const val = String(value);
            return this._entries.some(([k, v]) => k === key && v === val);
        }

        set(name, value) {
            const key = String(name);
            const val = String(value);
            let replaced = false;
            const next = [];
            for (const pair of this._entries) {
                if (pair[0] === key) {
                    if (!replaced) {
                        next.push([key, val]);
                        replaced = true;
                    }
                } else {
                    next.push(pair);
                }
            }
            if (!replaced) next.push([key, val]);
            this._entries = next;
            this._notify();
        }

        sort() {
            this._entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
            this._notify();
        }

        *keys() {
            for (const [k] of this._entries) yield k;
        }

        *values() {
            for (const [, v] of this._entries) yield v;
        }

        *entries() {
            for (const pair of this._entries) yield [pair[0], pair[1]];
        }

        [Symbol.iterator]() {
            return this.entries();
        }

        forEach(callback, thisArg) {
            if (typeof callback !== "function") {
                throw new TypeError("callback must be a function");
            }
            for (const [k, v] of this._entries) {
                callback.call(thisArg, v, k, this);
            }
        }

        toString() {
            return this._entries
                .map(([k, v]) => `${encodeFormComponent(k)}=${encodeFormComponent(v)}`)
                .join("&");
        }
    }

    class URL {
        constructor(input, base) {
            if (arguments.length < 1) {
                throw new TypeError("URL constructor requires at least 1 argument");
            }
            const rawInput = String(input);
            const rawBase = base === undefined ? undefined : String(base);
            const parsed = host.urlParse(rawInput, rawBase);
            this[URL_INTERNAL] = parsed;
            const params = new URLSearchParams(parsed.search);
            params[PARAMS_OWNER] = this;
            this._searchParams = params;
        }

        _update(component, value) {
            const next = host.urlUpdate(this[URL_INTERNAL].href, component, String(value));
            this[URL_INTERNAL] = next;
            if (component === "search" || component === "href") {
                this._searchParams._entries = parseQueryPairs(next.search);
            }
        }

        _syncFromParams() {
            const serialized = this._searchParams.toString();
            const next = host.urlUpdate(this[URL_INTERNAL].href, "search", serialized ? `?${serialized}` : "");
            this[URL_INTERNAL] = next;
        }

        get href() { return this[URL_INTERNAL].href; }
        set href(val) { this._update("href", val); }

        get origin() { return this[URL_INTERNAL].origin; }

        get protocol() { return this[URL_INTERNAL].protocol; }
        set protocol(val) { this._update("protocol", val); }

        get username() { return this[URL_INTERNAL].username; }
        set username(val) { this._update("username", val); }

        get password() { return this[URL_INTERNAL].password; }
        set password(val) { this._update("password", val); }

        get host() { return this[URL_INTERNAL].host; }
        set host(val) { this._update("host", val); }

        get hostname() { return this[URL_INTERNAL].hostname; }
        set hostname(val) { this._update("hostname", val); }

        get port() { return this[URL_INTERNAL].port; }
        set port(val) { this._update("port", val); }

        get pathname() { return this[URL_INTERNAL].pathname; }
        set pathname(val) { this._update("pathname", val); }

        get search() { return this[URL_INTERNAL].search; }
        set search(val) { this._update("search", val); }

        get searchParams() { return this._searchParams; }

        get hash() { return this[URL_INTERNAL].hash; }
        set hash(val) { this._update("hash", val); }

        toString() { return this[URL_INTERNAL].href; }
        toJSON() { return this[URL_INTERNAL].href; }

        static canParse(input, base) {
            try {
                host.urlParse(String(input), base === undefined ? undefined : String(base));
                return true;
            } catch {
                return false;
            }
        }

        static parse(input, base) {
            try {
                return new URL(input, base);
            } catch {
                return null;
            }
        }
    }

    function fileURLToPath(url) {
        const str = url instanceof URL ? url.href : assertString(url, "url");
        return host.urlFileToPath(str);
    }

    function pathToFileURL(p) {
        const urlStr = host.urlPathToFile(assertString(p, "path"));
        return new URL(urlStr);
    }

    const urlMod = Object.freeze({
        fileURLToPath,
        pathToFileURL,
        URL,
        URLSearchParams,
    });

    // -------------------------------------------------------------------------
    // node:path, node:path/posix, node:path/win32
    // -------------------------------------------------------------------------
    function validateFormatObj(obj) {
        if (obj === null || typeof obj !== "object") {
            throw new TypeError("The \"pathObject\" argument must be of type object");
        }
        const out = {};
        for (const key of ["root", "dir", "base", "ext", "name"]) {
            const val = obj[key];
            if (val !== undefined) {
                if (typeof val !== "string") {
                    throw new TypeError(`pathObject.${key} must be a string`);
                }
                out[key] = val;
            }
        }
        return out;
    }

    const pathInfo = host.pathInfo();

    function makePathDialect(isWin) {
        const sep = isWin ? "\\" : "/";
        const delimiter = isWin ? ";" : ":";
        return {
            sep,
            delimiter,
            join(...paths) {
                for (let i = 0; i < paths.length; i++) {
                    assertString(paths[i], "path");
                }
                return host.pathJoin(isWin, paths);
            },
            resolve(...paths) {
                for (let i = 0; i < paths.length; i++) {
                    assertString(paths[i], "path");
                }
                return host.pathResolve(isWin, paths);
            },
            normalize(p) {
                return host.pathNormalize(isWin, assertString(p, "path"));
            },
            relative(from, to) {
                return host.pathRelative(isWin, assertString(from, "from"), assertString(to, "to"));
            },
            dirname(p) {
                return host.pathDirname(isWin, assertString(p, "path"));
            },
            basename(p, suffix) {
                return host.pathBasename(
                    isWin,
                    assertString(p, "path"),
                    suffix === undefined ? undefined : assertString(suffix, "suffix"),
                );
            },
            extname(p) {
                return host.pathExtname(isWin, assertString(p, "path"));
            },
            parse(p) {
                return host.pathParse(isWin, assertString(p, "path"));
            },
            format(obj) {
                return host.pathFormat(isWin, validateFormatObj(obj));
            },
            isAbsolute(p) {
                return host.pathIsAbsolute(isWin, assertString(p, "path"));
            },
        };
    }

    const pathPosix = makePathDialect(false);
    const pathWin32 = makePathDialect(true);
    pathPosix.posix = pathPosix;
    pathPosix.win32 = pathWin32;
    pathWin32.posix = pathPosix;
    pathWin32.win32 = pathWin32;
    Object.freeze(pathPosix);
    Object.freeze(pathWin32);
    const pathMod = pathInfo.isWindows ? pathWin32 : pathPosix;

    // -------------------------------------------------------------------------
    // node:os
    // -------------------------------------------------------------------------
    const osMeta = host.osInfo();
    const osMod = Object.freeze({
        platform: () => osMeta.platform,
        arch: () => osMeta.arch,
        EOL: osMeta.EOL,
        homedir: () => host.osHomedir(),
        tmpdir: () => host.osTmpdir(),
        hostname: () => host.osHostname(),
        cpus: () => Array.from({ length: osMeta.cpusCount }, () => ({
            model: "unknown",
            speed: 0,
            times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 },
        })),
        endianness: () => osMeta.endianness,
        type: () => osMeta.type,
        machine: () => osMeta.machine,
    });

    // -------------------------------------------------------------------------
    // node:process
    // -------------------------------------------------------------------------
    const procMeta = host.processInfo();
    const envOverrides = new Map();
    const envDeleted = new Set();

    const processEnv = new Proxy(Object.create(null), {
        get(_target, prop) {
            if (typeof prop === "symbol") return undefined;
            if (envOverrides.has(prop)) return envOverrides.get(prop);
            if (envDeleted.has(prop)) return undefined;
            return host.processEnvGet(prop);
        },
        set(_target, prop, value) {
            if (typeof prop === "symbol") return false;
            envDeleted.delete(prop);
            envOverrides.set(prop, String(value));
            return true;
        },
        has(_target, prop) {
            if (typeof prop === "symbol") return false;
            if (envOverrides.has(prop)) return true;
            if (envDeleted.has(prop)) return false;
            return host.processEnvGet(prop) !== undefined;
        },
        deleteProperty(_target, prop) {
            if (typeof prop === "symbol") return false;
            envOverrides.delete(prop);
            envDeleted.add(prop);
            return true;
        },
        ownKeys() {
            const keys = new Set();
            for (const [k] of host.processEnvEntries()) {
                if (!envDeleted.has(k)) keys.add(k);
            }
            for (const k of envOverrides.keys()) {
                keys.add(k);
            }
            return Array.from(keys);
        },
        getOwnPropertyDescriptor(_target, prop) {
            if (typeof prop === "symbol") return undefined;
            let value;
            if (envOverrides.has(prop)) {
                value = envOverrides.get(prop);
            } else if (envDeleted.has(prop)) {
                return undefined;
            } else {
                value = host.processEnvGet(prop);
                if (value === undefined) return undefined;
            }
            return {
                value,
                writable: true,
                enumerable: true,
                configurable: true,
            };
        },
    });

    function hrtime(prev) {
        let nanos = BigInt(host.processHrtimeNanos());
        if (prev !== undefined) {
            if (!Array.isArray(prev) || prev.length !== 2) {
                throw new TypeError("The \"time\" argument must be an array of [seconds, nanoseconds]");
            }
            const prevNanos = BigInt(Math.trunc(prev[0])) * 1000000000n + BigInt(Math.trunc(prev[1]));
            nanos -= prevNanos;
        }
        const secs = nanos / 1000000000n;
        const rem = nanos % 1000000000n;
        return [Number(secs), Number(rem)];
    }
    hrtime.bigint = () => BigInt(host.processHrtimeNanos());

    function normalizeExitCode(code) {
        if (code === undefined || code === null) return undefined;
        const n = Number(code);
        if (!Number.isFinite(n)) {
            throw new TypeError("exitCode must be a finite integer");
        }
        return n | 0;
    }

    const processMod = {
        argv: Object.freeze(procMeta.argv.slice()),
        cwd: () => host.processCwd(),
        chdir: (dir) => host.processChdir(assertString(dir, "directory")),
        env: processEnv,
        exit: (code) => {
            const normalized = code === undefined ? undefined : normalizeExitCode(code);
            host.processExit(normalized);
        },
        platform: procMeta.platform,
        arch: procMeta.arch,
        pid: procMeta.pid,
        version: procMeta.version,
        versions: Object.freeze({ ...procMeta.versions }),
        hrtime,
    };
    Object.defineProperty(processMod, "exitCode", {
        enumerable: true,
        configurable: false,
        get() {
            return host.processGetExitCode();
        },
        set(val) {
            host.processSetExitCode(normalizeExitCode(val));
        },
    });

    // -------------------------------------------------------------------------
    // node:fs & node:fs/promises
    // -------------------------------------------------------------------------
    function toFsPath(pathLike) {
        if (pathLike instanceof URL) {
            return host.urlFileToPath(pathLike.href);
        }
        return assertString(pathLike, "path");
    }

    function readEncodingOption(options) {
        if (options === undefined || options === null) return null;
        if (typeof options === "string") return normalizeEncoding(options);
        if (typeof options === "object") {
            const rawEnc = ownOpt(options, "encoding");
            return rawEnc === undefined || rawEnc === null
                ? null
                : normalizeEncoding(rawEnc);
        }
        throw new TypeError("options must be a string or an object");
    }

    function writeOptions(options, defaultAppend) {
        if (options === undefined || options === null) {
            return { encoding: "utf8", append: defaultAppend };
        }
        if (typeof options === "string") {
            return { encoding: normalizeEncoding(options), append: defaultAppend };
        }
        if (typeof options === "object") {
            const rawEnc = ownOpt(options, "encoding");
            const encoding = rawEnc === undefined || rawEnc === null
                ? "utf8"
                : normalizeEncoding(rawEnc);
            const append = defaultAppend || ownOpt(options, "flag") === "a";
            return { encoding, append };
        }
        throw new TypeError("options must be a string or an object");
    }

    class Stats {
        constructor(raw) {
            this.size = raw.size;
            this.mtimeMs = raw.mtimeMs ?? 0;
            this.mtime = new Date(this.mtimeMs);
            this._isFile = raw.isFile;
            this._isDirectory = raw.isDirectory;
            this._isSymbolicLink = raw.isSymbolicLink;
        }
        isFile() { return this._isFile; }
        isDirectory() { return this._isDirectory; }
        isSymbolicLink() { return this._isSymbolicLink; }
    }

    class Dirent {
        constructor(raw) {
            this.name = raw.name;
            this._isFile = raw.isFile;
            this._isDirectory = raw.isDirectory;
            this._isSymbolicLink = raw.isSymbolicLink;
        }
        isFile() { return this._isFile; }
        isDirectory() { return this._isDirectory; }
        isSymbolicLink() { return this._isSymbolicLink; }
    }

    const fsConstants = Object.freeze({
        F_OK: 0,
        R_OK: 4,
        W_OK: 2,
        X_OK: 1,
        COPYFILE_EXCL: 1,
        COPYFILE_FICLONE: 2,
        COPYFILE_FICLONE_FORCE: 4,
    });

    const fsSyncFields = {
        readFileSync(path, options) {
            const enc = readEncodingOption(options);
            const bytes = host.fsReadFileSync(toFsPath(path));
            return enc === null ? toBuffer(bytes) : host.bufferEncode(bytes, enc);
        },
        writeFileSync(path, data, options) {
            const opts = writeOptions(options, false);
            host.fsWriteFileSync(toFsPath(path), toBytes(data, opts.encoding), opts.append);
        },
        appendFileSync(path, data, options) {
            const opts = writeOptions(options, true);
            host.fsWriteFileSync(toFsPath(path), toBytes(data, opts.encoding), opts.append);
        },
        existsSync(path) {
            return host.fsExistsSync(toFsPath(path));
        },
        statSync(path) {
            return new Stats(host.fsStatSync(toFsPath(path), true));
        },
        lstatSync(path) {
            return new Stats(host.fsStatSync(toFsPath(path), false));
        },
        readdirSync(path, options) {
            const withFileTypes = Boolean(options && typeof options === "object" && ownOpt(options, "withFileTypes"));
            const entries = host.fsReaddirSync(toFsPath(path));
            return withFileTypes ? entries.map((e) => new Dirent(e)) : entries.map((e) => e.name);
        },
        mkdirSync(path, options) {
            const recursive = Boolean(options && typeof options === "object" && ownOpt(options, "recursive"));
            host.fsMkdirSync(toFsPath(path), recursive);
        },
        rmSync(path, options) {
            const recursive = Boolean(options && typeof options === "object" && ownOpt(options, "recursive"));
            const force = Boolean(options && typeof options === "object" && ownOpt(options, "force"));
            host.fsRmSync(toFsPath(path), recursive, force);
        },
        unlinkSync(path) {
            host.fsUnlinkSync(toFsPath(path));
        },
        renameSync(oldPath, newPath) {
            host.fsRenameSync(toFsPath(oldPath), toFsPath(newPath));
        },
        copyFileSync(src, dest, mode = 0) {
            const overwrite = (Number(mode) & fsConstants.COPYFILE_EXCL) === 0;
            host.fsCopyFileSync(toFsPath(src), toFsPath(dest), overwrite);
        },
        realpathSync(path) {
            return host.fsRealpathSync(toFsPath(path));
        },
        mkdtempSync(prefix) {
            return host.fsMkdtempSync(assertString(prefix, "prefix"));
        },
        constants: fsConstants,
    };

    let fsPromisesMod;
    if (isV8) {
        fsPromisesMod = Object.freeze({
            async readFile(path, options) {
                const enc = readEncodingOption(options);
                const bytes = await host.fsReadFileAsync(toFsPath(path));
                return enc === null ? toBuffer(bytes) : host.bufferEncode(bytes, enc);
            },
            async writeFile(path, data, options) {
                const opts = writeOptions(options, false);
                await host.fsWriteFileAsync(toFsPath(path), toBytes(data, opts.encoding), opts.append);
            },
            async appendFile(path, data, options) {
                const opts = writeOptions(options, true);
                await host.fsWriteFileAsync(toFsPath(path), toBytes(data, opts.encoding), opts.append);
            },
            async stat(path) {
                return new Stats(await host.fsStatAsync(toFsPath(path), true));
            },
            async lstat(path) {
                return new Stats(await host.fsStatAsync(toFsPath(path), false));
            },
            async readdir(path, options) {
                const withFileTypes = Boolean(options && typeof options === "object" && ownOpt(options, "withFileTypes"));
                const entries = await host.fsReaddirAsync(toFsPath(path));
                return withFileTypes ? entries.map((e) => new Dirent(e)) : entries.map((e) => e.name);
            },
            async mkdir(path, options) {
                const recursive = Boolean(options && typeof options === "object" && ownOpt(options, "recursive"));
                await host.fsMkdirAsync(toFsPath(path), recursive);
            },
            async rm(path, options) {
                const recursive = Boolean(options && typeof options === "object" && ownOpt(options, "recursive"));
                const force = Boolean(options && typeof options === "object" && ownOpt(options, "force"));
                await host.fsRmAsync(toFsPath(path), recursive, force);
            },
            async unlink(path) {
                await host.fsUnlinkAsync(toFsPath(path));
            },
            async rename(oldPath, newPath) {
                await host.fsRenameAsync(toFsPath(oldPath), toFsPath(newPath));
            },
            async copyFile(src, dest, mode = 0) {
                const overwrite = (Number(mode) & fsConstants.COPYFILE_EXCL) === 0;
                await host.fsCopyFileAsync(toFsPath(src), toFsPath(dest), overwrite);
            },
            async realpath(path) {
                return await host.fsRealpathAsync(toFsPath(path));
            },
            async mkdtemp(prefix) {
                return await host.fsMkdtempAsync(assertString(prefix, "prefix"));
            },
            constants: fsConstants,
        });
        fsSyncFields.promises = fsPromisesMod;
    }

    const fsMod = Object.freeze(fsSyncFields);

    // -------------------------------------------------------------------------
    // node:child_process
    // -------------------------------------------------------------------------
    function parseChildArgs(file, argsOrOpts, maybeOpts) {
        const command = assertString(file, "command");
        const args = [];
        let options = objectCreate(null);
        if (argsOrOpts !== undefined && argsOrOpts !== null) {
            if (arrayIsArray(argsOrOpts)) {
                const argCount = argsOrOpts.length;
                if (!numberIsInteger(argCount) || argCount < 0 || argCount > MAX_CHILD_ARGS) {
                    throw new makeRangeError(`args length exceeds maximum (${MAX_CHILD_ARGS})`);
                }
                for (let i = 0; i < argCount; i++) {
                    const a = argsOrOpts[i];
                    if (typeof a !== "string") {
                        throw new makeTypeError("args must be an array of strings");
                    }
                    args[i] = a;
                }
                if (maybeOpts !== undefined && maybeOpts !== null) {
                    if (typeof maybeOpts !== "object") {
                        throw new makeTypeError("options must be an object");
                    }
                    options = maybeOpts;
                }
            } else if (typeof argsOrOpts === "object") {
                options = argsOrOpts;
            } else {
                throw new makeTypeError("args must be an array of strings");
            }
        }
        if (ownOpt(options, "shell")) {
            const err = new makeError("shell execution is disabled in vaeljs");
            defineDataProp(err, "code", "ERR_UNSUPPORTED");
            throw err;
        }
        let cwd;
        const rawCwd = ownOpt(options, "cwd");
        if (rawCwd !== undefined) {
            cwd = toFsPath(rawCwd);
        }
        let env;
        const rawEnv = ownOpt(options, "env");
        if (rawEnv !== undefined) {
            if (rawEnv === null || typeof rawEnv !== "object") {
                throw new makeTypeError("options.env must be an object");
            }
            const keys = objectKeys(rawEnv);
            if (keys.length > MAX_CHILD_ENV) {
                throw new makeRangeError(`options.env exceeds maximum entries (${MAX_CHILD_ENV})`);
            }
            env = [];
            let envIdx = 0;
            for (let i = 0; i < keys.length; i++) {
                const key = keys[i];
                const val = ownOpt(rawEnv, key);
                if (val !== undefined && val !== null) {
                    env[envIdx++] = [String(key), String(val)];
                }
            }
        }
        let input;
        const rawInput = ownOpt(options, "input");
        if (rawInput !== undefined && rawInput !== null) {
            input = toBytes(rawInput, "utf8");
        }
        let timeoutMs;
        const rawTimeout = ownOpt(options, "timeout");
        if (rawTimeout !== undefined) {
            if (typeof rawTimeout !== "number" || !numberIsFinite(rawTimeout) || rawTimeout < 0) {
                throw new makeRangeError("options.timeout must be a non-negative finite number");
            }
            timeoutMs = mathTrunc(rawTimeout);
        }
        let maxBuffer;
        const rawMaxBuffer = ownOpt(options, "maxBuffer");
        if (rawMaxBuffer !== undefined) {
            if (
                typeof rawMaxBuffer !== "number" ||
                !numberIsFinite(rawMaxBuffer) ||
                rawMaxBuffer < 0 ||
                rawMaxBuffer > MAX_MAX_BUFFER
            ) {
                throw new makeRangeError(
                    `options.maxBuffer must be a non-negative finite number <= ${MAX_MAX_BUFFER}`,
                );
            }
            maxBuffer = mathTrunc(rawMaxBuffer);
        }
        let encoding = null;
        const rawEncoding = ownOpt(options, "encoding");
        if (rawEncoding !== undefined && rawEncoding !== null && rawEncoding !== "buffer") {
            encoding = normalizeEncoding(rawEncoding);
        }
        const nativeOpts = objectCreate(null);
        nativeOpts.cwd = cwd;
        nativeOpts.env = env;
        nativeOpts.input = input;
        nativeOpts.timeoutMs = timeoutMs;
        nativeOpts.maxBuffer = maxBuffer;
        return {
            command,
            args,
            nativeOpts,
            encoding,
        };
    }

    function decodeStdio(bytes, encoding) {
        return encoding === null ? toBuffer(bytes) : host.bufferEncode(bytes, encoding);
    }

    function formatChildError(res, stdout, stderr) {
        let message = res.errorMessage;
        if (!message) {
            if (res.timedOut) {
                message = "Command timed out";
            } else if (res.signal) {
                message = `Command failed with signal ${res.signal}`;
            } else {
                message = `Command failed with exit code ${res.status}`;
            }
        }
        const err = new makeError(message);
        defineDataProp(err, "code", res.errorCode ?? (res.timedOut ? "ETIMEDOUT" : undefined));
        defineDataProp(err, "status", res.status);
        defineDataProp(err, "signal", res.signal);
        defineDataProp(err, "stdout", stdout);
        defineDataProp(err, "stderr", stderr);
        defineDataProp(err, "timedOut", res.timedOut);
        return err;
    }

    const childSyncFields = {
        spawnSync(command, argsOrOpts, maybeOpts) {
            const parsed = parseChildArgs(command, argsOrOpts, maybeOpts);
            const res = host.childSpawnSync(parsed.command, parsed.args, parsed.nativeOpts);
            const stdout = decodeStdio(res.stdout, parsed.encoding);
            const stderr = decodeStdio(res.stderr, parsed.encoding);
            let error;
            if (res.errorCode || res.timedOut) {
                error = formatChildError(res, stdout, stderr);
            }
            return {
                status: res.status,
                signal: res.signal,
                stdout,
                stderr,
                error,
                timedOut: res.timedOut,
            };
        },
        execFileSync(file, argsOrOpts, maybeOpts) {
            const parsed = parseChildArgs(file, argsOrOpts, maybeOpts);
            const res = host.childSpawnSync(parsed.command, parsed.args, parsed.nativeOpts);
            const stdout = decodeStdio(res.stdout, parsed.encoding);
            const stderr = decodeStdio(res.stderr, parsed.encoding);
            if (res.errorCode || res.timedOut || res.status !== 0) {
                throw formatChildError(res, stdout, stderr);
            }
            return stdout;
        },
    };

    if (isV8) {
        childSyncFields.execFile = function execFile(file, argsOrOptsOrCb, optsOrCb, maybeCb) {
            let argsOrOpts = argsOrOptsOrCb;
            let maybeOpts = optsOrCb;
            let callback = null;
            if (typeof maybeCb === "function") {
                callback = maybeCb;
            } else if (typeof optsOrCb === "function") {
                callback = optsOrCb;
                maybeOpts = undefined;
            } else if (typeof argsOrOptsOrCb === "function") {
                callback = argsOrOptsOrCb;
                argsOrOpts = undefined;
                maybeOpts = undefined;
            }
            const parsed = parseChildArgs(file, argsOrOpts, maybeOpts);
            const promise = host.childExecFileAsync(parsed.command, parsed.args, parsed.nativeOpts)
                .then((res) => {
                    const stdout = decodeStdio(res.stdout, parsed.encoding);
                    const stderr = decodeStdio(res.stderr, parsed.encoding);
                    if (res.errorCode || res.timedOut || res.status !== 0) {
                        throw formatChildError(res, stdout, stderr);
                    }
                    return { stdout, stderr };
                });
            if (callback !== null) {
                promise.then(
                    ({ stdout, stderr }) => callback(null, stdout, stderr),
                    (err) => callback(err, err.stdout ?? "", err.stderr ?? ""),
                );
                return undefined;
            }
            return promise;
        };
    }

    const childProcessMod = Object.freeze(childSyncFields);

    // -------------------------------------------------------------------------
    // node:crypto
    // -------------------------------------------------------------------------
    class Hash {
        constructor(handle) {
            this._handle = handle;
            this._digested = false;
        }
        update(data, inputEncoding) {
            if (this._digested) {
                throw new Error("Digest already called");
            }
            host.cryptoHashUpdate(this._handle, toBytes(data, inputEncoding));
            return this;
        }
        digest(encoding) {
            if (this._digested) {
                throw new Error("Digest already called");
            }
            this._digested = true;
            const bytes = host.cryptoHashDigest(this._handle);
            if (encoding === undefined || encoding === null || encoding === "buffer") {
                return toBuffer(bytes);
            }
            return host.bufferEncode(bytes, normalizeEncoding(encoding));
        }
    }

    const cryptoMod = Object.freeze({
        randomBytes(size, callback) {
            if (!Number.isInteger(size) || size < 0) {
                throw new RangeError("The value of \"size\" is out of range");
            }
            const buf = toBuffer(host.cryptoRandomBytes(size));
            if (typeof callback === "function") {
                Promise.resolve().then(() => callback(null, buf));
                return undefined;
            }
            return buf;
        },
        randomUUID() {
            return host.cryptoRandomUUID();
        },
        createHash(algorithm) {
            return new Hash(host.cryptoHashCreate(assertString(algorithm, "algorithm")));
        },
        createHmac(algorithm, key) {
            return new Hash(host.cryptoHmacCreate(assertString(algorithm, "algorithm"), toBytes(key, "utf8")));
        },
        pbkdf2Sync(password, salt, iterations, keylen, digest) {
            if (!numberIsInteger(iterations) || iterations <= 0 || iterations > 0xffffffff) {
                throw new makeRangeError("iterations must be a positive 32-bit integer");
            }
            if (!numberIsInteger(keylen) || keylen < 0) {
                throw new makeRangeError("keylen must be a non-negative integer");
            }
            const pwBytes = toBytes(password, "utf8");
            const saltBytes = toBytes(salt, "utf8");
            const alg = assertString(digest, "digest");
            return toBuffer(host.cryptoPbkdf2Sync(pwBytes, saltBytes, iterations, keylen, alg));
        },
        timingSafeEqual(a, b) {
            const va = readViewSlice(a);
            const vb = readViewSlice(b);
            if (va === null || vb === null) {
                throw new makeTypeError("The \"buf1\" and \"buf2\" arguments must be TypedArray or DataView instances");
            }
            const sa = new makeUint8Array(taByteLengthOf(va));
            typedArraySet(sa, va);
            const sb = new makeUint8Array(taByteLengthOf(vb));
            typedArraySet(sb, vb);
            return host.cryptoTimingSafeEqual(sa, sb);
        },
    });

    const result = {
        "node:path": pathMod,
        "node:path/posix": pathPosix,
        "node:path/win32": pathWin32,
        "node:os": osMod,
        "node:process": processMod,
        "node:fs": fsMod,
        "node:child_process": childProcessMod,
        "node:buffer": bufferMod,
        "node:url": urlMod,
        "node:crypto": cryptoMod,
    };
    if (isV8) {
        result["node:fs/promises"] = fsPromisesMod;
    }
    return result;
}
