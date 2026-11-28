// The `vael:fetch` built-in module factory, evaluated inside `vael.js` only when
// the `vael-fetch` Cargo feature is enabled.
(function createVaelFetch(deps) {
    "use strict";

    const ops = deps.ops;
    const opFetchSync = ops.op_vael_fetch_sync;
    const opFetchAsync = ops.op_vael_fetch_async;
    const opDownloadSync = ops.op_vael_fetch_download_sync;
    const opDownloadStart = ops.op_vael_fetch_download_start;
    const opDownloadNext = ops.op_vael_fetch_download_next;
    const opDownloadClose = ops.op_vael_fetch_download_close;
    const opDecodeUtf8 = ops.op_vael_fetch_decode_utf8;

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeUint8Array = Uint8Array;
    const freeze = Object.freeze;
    const defineProperty = Object.defineProperty;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const objectCreate = Object.create;
    const hasOwn = Object.hasOwn;
    const jsonParse = JSON.parse;
    const stringToLowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);

    const typedArrayProto = getPrototypeOf(Uint8Array.prototype);
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);
    const typedArrayIndexOf = Function.prototype.call.bind(typedArrayProto.indexOf);
    const typedArrayByteLength = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayProto, "byteLength").get,
    );

    const fetchResponseToken = Symbol("vaeljs.fetch.Response");

    function defineOwn(target, key, value, enumerable, writable) {
        defineProperty(target, key, {
            __proto__: null,
            value,
            writable,
            enumerable,
            configurable: true,
        });
    }

    function copyBytes(bytes) {
        typedArrayIndexOf(bytes, 0, 1 / 0);
        const len = typedArrayByteLength(bytes);
        const copy = new makeUint8Array(len);
        typedArraySet(copy, bytes);
        return copy;
    }

    function buildCollapsedHeadersRecord(pairs) {
        const record = objectCreate(null);
        for (let i = 0; i < pairs.length; i += 1) {
            const name = stringToLowerCase(pairs[i][0]);
            const val = pairs[i][1];
            if (hasOwn(record, name)) {
                const sep = name === "cookie" ? "; " : ", ";
                defineOwn(record, name, record[name] + sep + val, true, true);
            } else {
                defineOwn(record, name, val, true, true);
            }
        }
        return freeze(record);
    }

    /**
     * Error thrown by `vael:fetch` operations for URL, option, SSRF, network,
     * timeout, HTTP status, checksum, I/O, UTF-8, or JSON failures.
     */
    class FetchError extends makeError {
        constructor(code, message, status) {
            super(message);
            defineOwn(this, "name", "FetchError", false, true);
            defineOwn(this, "code", code, true, false);
            if (status !== undefined) {
                defineOwn(this, "status", status, true, false);
            }
        }
    }
    freeze(FetchError.prototype);
    freeze(FetchError);

    function throwFetch(outcome) {
        if (outcome.name === "TypeError") {
            throw new makeTypeError(outcome.message);
        }
        throw new FetchError(outcome.code, outcome.message, outcome.status);
    }

    /**
     * Immutable HTTP response returned by `fetch` and `fetchSync`.
     */
    class FetchResponse {
        #status;
        #statusText;
        #ok;
        #url;
        #redirected;
        #headersRecord;
        #bodyBytes;

        constructor(token, data) {
            if (token !== fetchResponseToken) {
                throw new makeTypeError("FetchResponse cannot be constructed directly");
            }
            this.#status = data.status;
            this.#statusText = data.statusText;
            this.#ok = data.ok;
            this.#url = data.url;
            this.#redirected = data.redirected;
            this.#headersRecord = buildCollapsedHeadersRecord(data.headers);
            this.#bodyBytes = data.body;
            freeze(this);
        }

        get status() {
            return this.#status;
        }

        get statusText() {
            return this.#statusText;
        }

        get ok() {
            return this.#ok;
        }

        get url() {
            return this.#url;
        }

        get redirected() {
            return this.#redirected;
        }

        get headers() {
            return this.#headersRecord;
        }

        get body() {
            return copyBytes(this.#bodyBytes);
        }

        /**
         * Case-insensitive lookup for a response header value.
         *
         * @param {string} name
         * @returns {string | undefined}
         */
        header(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("header takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            return hasOwn(this.#headersRecord, lower) ? this.#headersRecord[lower] : undefined;
        }

        /**
         * Return a fresh `Uint8Array` copy of the response body.
         *
         * @returns {Uint8Array}
         */
        bytes() {
            if (arguments.length !== 0) {
                throw new makeTypeError("bytes takes no arguments");
            }
            return copyBytes(this.#bodyBytes);
        }

        /**
         * Decode the response body as strict UTF-8 text.
         *
         * @returns {string}
         * @throws {FetchError} with `code` `VAEL_FETCH_INVALID_UTF8`.
         */
        text() {
            if (arguments.length !== 0) {
                throw new makeTypeError("text takes no arguments");
            }
            const res = opDecodeUtf8(this.#bodyBytes);
            if (!res.ok) throwFetch(res);
            return res.value;
        }

        /**
         * Parse the response body as JSON.
         *
         * @returns {unknown}
         * @throws {FetchError} with `code` `VAEL_FETCH_INVALID_JSON`.
         */
        json() {
            if (arguments.length !== 0) {
                throw new makeTypeError("json takes no arguments");
            }
            const res = opDecodeUtf8(this.#bodyBytes);
            if (!res.ok) {
                throw new FetchError("VAEL_FETCH_INVALID_JSON", "response body is not valid JSON");
            }
            try {
                return jsonParse(res.value);
            } catch (error) {
                const reason =
                    error && typeof error.message === "string" ? error.message : "invalid JSON";
                throw new FetchError(
                    "VAEL_FETCH_INVALID_JSON",
                    `response body is not valid JSON: ${reason}`,
                );
            }
        }
    }
    freeze(FetchResponse.prototype);
    freeze(FetchResponse);

    function freezeDownloadResult(raw) {
        return freeze({
            path: raw.path,
            status: raw.status,
            url: raw.url,
            bytesWritten: raw.bytesWritten,
            sha256: raw.sha256,
            contentType: raw.contentType,
            etag: raw.etag,
        });
    }

    function extractOwnOnProgress(options) {
        if (options === undefined) return undefined;
        const desc = getOwnPropertyDescriptor(options, "onProgress");
        return desc !== undefined ? desc.value : undefined;
    }

    /**
     * Perform a synchronous SSRF-guarded HTTP/HTTPS request.
     *
     * @param {string} url
     * @param {object} [options]
     * @returns {FetchResponse}
     */
    function fetchSync(url, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("fetchSync takes one or two arguments");
        }
        const res = opFetchSync(url, options);
        if (!res.ok) throwFetch(res);
        return new FetchResponse(fetchResponseToken, res.value);
    }

    /**
     * Perform an asynchronous SSRF-guarded HTTP/HTTPS request on the blocking
     * pool without blocking V8's event loop.
     *
     * @param {string} url
     * @param {object} [options]
     * @returns {Promise<FetchResponse>}
     */
    async function fetch(url, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("fetch takes one or two arguments");
        }
        const res = await opFetchAsync(url, options);
        if (!res.ok) throwFetch(res);
        return new FetchResponse(fetchResponseToken, res.value);
    }

    /**
     * Synchronously stream an HTTP/HTTPS GET download to `destPath` atomically.
     *
     * @param {string} url
     * @param {string} destPath
     * @param {object} [options]
     * @returns {object}
     */
    function downloadSync(url, destPath, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("downloadSync takes two or three arguments");
        }
        const res = opDownloadSync(url, destPath, options);
        if (!res.ok) throwFetch(res);
        const onProgress = extractOwnOnProgress(options);
        if (onProgress !== undefined && res.progress !== undefined) {
            for (let i = 0; i < res.progress.length; i += 1) {
                const p = res.progress[i];
                onProgress(
                    freeze({
                        bytesWritten: p.bytesWritten,
                        totalBytes: p.totalBytes,
                    }),
                );
            }
        }
        return freezeDownloadResult(res.value);
    }

    /**
     * Asynchronously stream an HTTP/HTTPS GET download to `destPath` atomically
     * on the blocking pool while delivering `onProgress` updates.
     *
     * @param {string} url
     * @param {string} destPath
     * @param {object} [options]
     * @returns {Promise<object>}
     */
    async function download(url, destPath, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("download takes two or three arguments");
        }
        const start = opDownloadStart(url, destPath, options);
        if (!start.ok) throwFetch(start);
        const rid = start.id;
        const onProgress = extractOwnOnProgress(options);
        try {
            while (true) {
                const ev = await opDownloadNext(rid);
                if (!ev.ok) throwFetch(ev);
                if (ev.kind === "progress") {
                    if (onProgress !== undefined) {
                        onProgress(
                            freeze({
                                bytesWritten: ev.value.bytesWritten,
                                totalBytes: ev.value.totalBytes,
                            }),
                        );
                    }
                } else if (ev.kind === "done") {
                    return freezeDownloadResult(ev.value);
                }
            }
        } finally {
            opDownloadClose(rid);
        }
    }

    return freeze({
        FetchError,
        download,
        downloadSync,
        fetch,
        fetchSync,
    });
})
