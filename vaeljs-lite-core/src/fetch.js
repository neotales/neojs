(function createVaelFetch(host) {
    "use strict";

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeString = String;
    const makeUint8Array = Uint8Array;
    const freeze = Object.freeze;
    const defineProperty = Object.defineProperty;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const objectCreate = Object.create;
    const callBind = Function.prototype.call.bind(Function.prototype.bind);
    const fnCallBind = (fn) => callBind(Function.prototype.call, fn);
    const hasOwn = fnCallBind(Object.prototype.hasOwnProperty);
    const jsonParse = JSON.parse;
    const stringToLowerCase = fnCallBind(String.prototype.toLowerCase);

    const typedArrayProto = getPrototypeOf(makeUint8Array.prototype);
    const typedArraySet = fnCallBind(typedArrayProto.set);
    const typedArrayByteLength = fnCallBind(
        getOwnPropertyDescriptor(typedArrayProto, "byteLength").get,
    );
    const typedArrayToStringTag = fnCallBind(
        getOwnPropertyDescriptor(typedArrayProto, Symbol.toStringTag).get,
    );
    const typedArrayIndexOf = fnCallBind(typedArrayProto.indexOf);

    const makeWeakMap = WeakMap;
    const weakMapGet = fnCallBind(makeWeakMap.prototype.get);
    const weakMapSet = fnCallBind(makeWeakMap.prototype.set);

    const hostFetchSync = host.fetchSync;
    const hostDownloadSync = host.downloadSync;
    const hostDecodeUtf8Strict = host.decodeUtf8Strict;

    const responseBytes = new makeWeakMap();
    const responseHeaders = new makeWeakMap();
    const responseRawHeaders = new makeWeakMap();

    function defineOwn(target, key, value, enumerable = true, writable = true) {
        const desc = objectCreate(null);
        desc.value = value;
        desc.writable = writable;
        desc.enumerable = enumerable;
        desc.configurable = true;
        defineProperty(target, key, desc);
    }

    function copyBytes(bytes) {
        let tag;
        try {
            tag = typedArrayToStringTag(bytes);
        } catch {
            throw new makeTypeError("value must be a Uint8Array");
        }
        if (tag !== "Uint8Array") {
            throw new makeTypeError("value must be a Uint8Array");
        }
        typedArrayIndexOf(bytes, 0, 1 / 0);
        const len = typedArrayByteLength(bytes);
        const copy = new makeUint8Array(len);
        typedArraySet(copy, bytes);
        return copy;
    }

    function buildCollapsedHeadersRecord(pairs) {
        const record = objectCreate(null);
        for (let i = 0; i < pairs.length; i++) {
            const pair = pairs[i];
            const name = stringToLowerCase(pair[0]);
            const val = pair[1];
            if (hasOwn(record, name)) {
                const sep = name === "cookie" ? "; " : ", ";
                defineOwn(record, name, record[name] + sep + val, true, true);
            } else {
                defineOwn(record, name, val, true, true);
            }
        }
        return freeze(record);
    }

    class FetchError extends makeError {
        constructor(code = "VAEL_FETCH_NETWORK", message = "", status = undefined, url = undefined) {
            super(message === undefined ? "" : makeString(message));
            defineOwn(this, "name", "FetchError", false, true);
            defineOwn(
                this,
                "code",
                code === undefined ? "VAEL_FETCH_NETWORK" : makeString(code),
                true,
                false,
            );
            if (status !== undefined) {
                defineOwn(this, "status", status, true, false);
            }
            if (url !== undefined) {
                defineOwn(this, "url", makeString(url), true, false);
            }
        }
    }
    freeze(FetchError.prototype);
    freeze(FetchError);

    function makeFetchError(err) {
        return new FetchError(
            hasOwn(err, "code") ? err.code : undefined,
            hasOwn(err, "message") ? err.message : undefined,
            hasOwn(err, "status") ? err.status : undefined,
            hasOwn(err, "url") ? err.url : undefined,
        );
    }

    class FetchResponse {
        constructor(status, statusText, ok, url, redirected, headersRecord, rawPairs, rawBody) {
            weakMapSet(responseBytes, this, rawBody);
            weakMapSet(responseHeaders, this, headersRecord);
            weakMapSet(responseRawHeaders, this, rawPairs);
            defineOwn(this, "status", status, true, false);
            defineOwn(this, "statusText", statusText, true, false);
            defineOwn(this, "ok", ok, true, false);
            defineOwn(this, "url", url, true, false);
            defineOwn(this, "redirected", redirected, true, false);
            defineOwn(this, "headers", headersRecord, true, false);
            const bodyDesc = objectCreate(null);
            bodyDesc.get = function body() {
                const bytes = weakMapGet(responseBytes, this);
                if (bytes === undefined) {
                    throw new makeTypeError("invalid FetchResponse receiver");
                }
                return copyBytes(bytes);
            };
            bodyDesc.enumerable = true;
            bodyDesc.configurable = true;
            defineProperty(this, "body", bodyDesc);
            freeze(this);
        }

        header(name) {
            const headers = weakMapGet(responseHeaders, this);
            if (headers === undefined) {
                throw new makeTypeError("invalid FetchResponse receiver");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            return hasOwn(headers, lower) ? headers[lower] : undefined;
        }

        headerValues(name) {
            const pairs = weakMapGet(responseRawHeaders, this);
            if (pairs === undefined) {
                throw new makeTypeError("invalid FetchResponse receiver");
            }
            if (arguments.length !== 1) {
                throw new makeTypeError("headerValues takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            const out = [];
            for (let i = 0; i < pairs.length; i++) {
                const pair = pairs[i];
                if (pair[0] === lower) {
                    defineOwn(out, out.length, pair[1], true, true);
                }
            }
            return out;
        }

        bytes() {
            const raw = weakMapGet(responseBytes, this);
            if (raw === undefined) {
                throw new makeTypeError("invalid FetchResponse receiver");
            }
            return copyBytes(raw);
        }

        text() {
            const raw = weakMapGet(responseBytes, this);
            if (raw === undefined) {
                throw new makeTypeError("invalid FetchResponse receiver");
            }
            const res = hostDecodeUtf8Strict(raw);
            if (!res.ok) {
                throw makeFetchError(res);
            }
            return res.text;
        }

        json() {
            const raw = weakMapGet(responseBytes, this);
            if (raw === undefined) {
                throw new makeTypeError("invalid FetchResponse receiver");
            }
            const decoded = hostDecodeUtf8Strict(raw);
            if (!decoded.ok) {
                throw makeFetchError(decoded);
            }
            try {
                return jsonParse(decoded.text);
            } catch (err) {
                const msg = err && typeof err.message === "string"
                    ? `response body is not valid JSON: ${err.message}`
                    : "response body is not valid JSON";
                throw new FetchError("VAEL_FETCH_INVALID_JSON", msg);
            }
        }
    }
    freeze(FetchResponse.prototype);
    freeze(FetchResponse);

    function fetchSync(url, options) {
        if (typeof url !== "string") {
            throw new makeTypeError("url must be a string");
        }
        const res = hostFetchSync(url, options);
        if (!res.ok) {
            throw makeFetchError(res);
        }
        const rawPairs = [];
        for (let i = 0; i < res.headers.length; i++) {
            const pair = res.headers[i];
            defineOwn(
                rawPairs,
                rawPairs.length,
                [stringToLowerCase(pair[0]), pair[1]],
                true,
                true,
            );
        }
        const headersRecord = buildCollapsedHeadersRecord(res.headers);
        return new FetchResponse(
            res.status,
            res.statusText,
            res.isOk,
            res.url,
            res.redirected,
            headersRecord,
            rawPairs,
            res.body,
        );
    }

    function downloadSync(url, destPath, options) {
        if (typeof url !== "string") {
            throw new makeTypeError("url must be a string");
        }
        if (typeof destPath !== "string") {
            throw new makeTypeError("destPath must be a string");
        }
        const res = hostDownloadSync(url, destPath, options);
        if (!res.ok) {
            throw makeFetchError(res);
        }
        const out = objectCreate(null);
        defineOwn(out, "path", res.path, true, false);
        defineOwn(out, "status", res.status, true, false);
        defineOwn(out, "url", res.url, true, false);
        defineOwn(out, "bytesWritten", res.bytesWritten, true, false);
        defineOwn(out, "sha256", res.sha256, true, false);
        defineOwn(out, "contentType", res.contentType, true, false);
        defineOwn(out, "etag", res.etag, true, false);
        return freeze(out);
    }

    const mod = objectCreate(null);
    defineOwn(mod, "FetchError", FetchError, true, false);
    defineOwn(mod, "downloadSync", downloadSync, true, false);
    defineOwn(mod, "fetchSync", fetchSync, true, false);
    return freeze(mod);
})
