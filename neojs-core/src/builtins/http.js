// The `neo:http` built-in module factory, evaluated inside `neo.js` only when
// the `neo-http` Cargo feature is enabled.
(function createNeoHttp(deps) {
    "use strict";

    const ops = deps.ops;
    const opDefaultMaxBodyBytes = ops.op_neo_http_default_max_body_bytes;
    const opRandomId = ops.op_neo_http_random_id;
    const opRouterCreate = ops.op_neo_http_router_create;
    const opRouterAdd = ops.op_neo_http_router_add;
    const opRouterMatch = ops.op_neo_http_router_match;
    const opStaticResolve = ops.op_neo_http_static_resolve;
    const opParseCookies = ops.op_neo_http_parse_cookies;
    const opFormatSetCookie = ops.op_neo_http_format_set_cookie;
    const opValidateHeader = ops.op_neo_http_validate_header;
    const opEncodeUtf8 = ops.op_neo_http_encode_utf8;
    const opDecodeUtf8 = ops.op_neo_http_decode_utf8;
    const opParseQuery = ops.op_neo_http_parse_query;
    const opListen = ops.op_neo_http_listen;
    const opNextRequest = ops.op_neo_http_next_request;
    const opTakeRequest = ops.op_neo_http_take_request;
    const opRespond = ops.op_neo_http_respond;
    const opClose = ops.op_neo_http_close;

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const makeUint8Array = Uint8Array;
    const freeze = Object.freeze;
    const defineProperty = Object.defineProperty;
    const hasOwn = Object.hasOwn;
    const objectEntries = Object.entries;
    const objectCreate = Object.create;
    const arrayIsArray = Array.isArray;
    const isInteger = Number.isInteger;
    const promiseResolve = Promise.resolve.bind(Promise);
    const promiseAllSettled = Promise.allSettled.bind(Promise);
    const jsonStringify = JSON.stringify;
    const jsonParse = JSON.parse;
    const stringToLowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    const stringToUpperCase = Function.prototype.call.bind(String.prototype.toUpperCase);
    const stringTrim = Function.prototype.call.bind(String.prototype.trim);
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const stringIndexOf = Function.prototype.call.bind(String.prototype.indexOf);
    const stringIncludes = Function.prototype.call.bind(String.prototype.includes);
    const stringStartsWith = Function.prototype.call.bind(String.prototype.startsWith);
    const stringEndsWith = Function.prototype.call.bind(String.prototype.endsWith);
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);

    const DEFAULT_MAX_BODY_BYTES = opDefaultMaxBodyBytes();

    function throwFailure(result) {
        let err;
        if (result.name === "TypeError") {
            err = new makeTypeError(result.message);
        } else if (result.name === "RangeError") {
            err = new makeRangeError(result.message);
        } else {
            err = new makeError(result.message);
            if (result.name) {
                err.name = result.name;
            }
        }
        if (result.code !== undefined) {
            err.code = result.code;
        }
        throw err;
    }

    function unwrap(result) {
        if (!result || result.ok !== true) {
            throwFailure(result || { name: "Error", message: "HTTP host operation failed" });
        }
        return result;
    }

    function copyBytes(bytes) {
        const copy = new makeUint8Array(bytes.byteLength);
        typedArraySet(copy, bytes);
        return copy;
    }

    function encodeUtf8(str) {
        return unwrap(opEncodeUtf8(str)).bytes;
    }

    function decodeUtf8(bytes) {
        return unwrap(opDecodeUtf8(bytes)).text;
    }

    function validateHeader(name, value) {
        unwrap(opValidateHeader(name, value));
    }

    function buildSafeRecord(pairs) {
        const record = {};
        for (let i = 0; i < pairs.length; i++) {
            const pair = pairs[i];
            defineProperty(record, pair[0], {
                value: pair[1],
                writable: true,
                enumerable: true,
                configurable: true,
            });
        }
        return freeze(record);
    }

    function buildCollapsedHeadersRecord(pairs) {
        const record = {};
        for (let i = 0; i < pairs.length; i++) {
            const name = stringToLowerCase(pairs[i][0]);
            const val = pairs[i][1];
            if (hasOwn(record, name)) {
                const sep = name === "cookie" ? "; " : ", ";
                defineProperty(record, name, {
                    value: record[name] + sep + val,
                    writable: true,
                    enumerable: true,
                    configurable: true,
                });
            } else {
                defineProperty(record, name, {
                    value: val,
                    writable: true,
                    enumerable: true,
                    configurable: true,
                });
            }
        }
        return freeze(record);
    }

    function defaultStatusText(status) {
        switch (status) {
            case 400:
                return "Bad Request";
            case 401:
                return "Unauthorized";
            case 403:
                return "Forbidden";
            case 404:
                return "Not Found";
            case 405:
                return "Method Not Allowed";
            case 409:
                return "Conflict";
            case 413:
                return "Payload Too Large";
            case 422:
                return "Unprocessable Entity";
            case 429:
                return "Too Many Requests";
            case 500:
                return "Internal Server Error";
            case 502:
                return "Bad Gateway";
            case 503:
                return "Service Unavailable";
            default:
                return `HTTP ${status}`;
        }
    }

    class HttpError extends Error {
        constructor(status, message, code, details) {
            if (!isInteger(status) || status < 100 || status > 599) {
                throw new makeRangeError("HttpError status must be an integer between 100 and 599");
            }
            const msg = message !== undefined ? String(message) : defaultStatusText(status);
            super(msg);
            this.name = "HttpError";
            this.status = status;
            if (code !== undefined) {
                if (typeof code !== "string") {
                    throw new makeTypeError("HttpError code must be a string");
                }
                this.code = code;
            }
            if (details !== undefined) {
                this.details = details;
            }
        }
    }

    function httpError(status, message, code, details) {
        return new HttpError(status, message, code, details);
    }

    function badRequest(message = "Bad Request", code = "NEO_HTTP_BAD_REQUEST") {
        return new HttpError(400, message, code);
    }

    function unauthorized(message = "Unauthorized", code = "NEO_HTTP_UNAUTHORIZED") {
        return new HttpError(401, message, code);
    }

    function forbidden(message = "Forbidden", code = "NEO_HTTP_FORBIDDEN") {
        return new HttpError(403, message, code);
    }

    function notFound(message = "Not Found", code = "NEO_HTTP_NOT_FOUND") {
        return new HttpError(404, message, code);
    }

    function conflict(message = "Conflict", code = "NEO_HTTP_CONFLICT") {
        return new HttpError(409, message, code);
    }

    function methodNotAllowed(allow, message = "Method Not Allowed") {
        const err = new HttpError(405, message, "NEO_HTTP_METHOD_NOT_ALLOWED");
        if (allow !== undefined) {
            if (!arrayIsArray(allow)) {
                throw new makeTypeError("allow must be an array of HTTP method strings");
            }
            const cleaned = [];
            for (let i = 0; i < allow.length; i++) {
                const m = allow[i];
                if (typeof m !== "string" || m.length === 0) {
                    throw new makeTypeError("each entry in allow must be a non-empty string");
                }
                cleaned.push(m);
            }
            if (cleaned.length > 0) {
                const allowHeader = cleaned.join(", ");
                validateHeader("allow", allowHeader);
                err.headers = freeze({ allow: allowHeader });
            }
        }
        return err;
    }

    class QueryParams {
        #entries;

        constructor(entries) {
            this.#entries = entries;
            freeze(this);
        }

        get size() {
            return this.#entries.length;
        }

        get(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("query parameter name must be a string");
            }
            for (let i = 0; i < this.#entries.length; i++) {
                if (this.#entries[i][0] === name) {
                    return this.#entries[i][1];
                }
            }
            return null;
        }

        getAll(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("query parameter name must be a string");
            }
            const out = [];
            for (let i = 0; i < this.#entries.length; i++) {
                if (this.#entries[i][0] === name) {
                    out.push(this.#entries[i][1]);
                }
            }
            return out;
        }

        has(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("query parameter name must be a string");
            }
            for (let i = 0; i < this.#entries.length; i++) {
                if (this.#entries[i][0] === name) {
                    return true;
                }
            }
            return false;
        }

        *keys() {
            for (let i = 0; i < this.#entries.length; i++) {
                yield this.#entries[i][0];
            }
        }

        *values() {
            for (let i = 0; i < this.#entries.length; i++) {
                yield this.#entries[i][1];
            }
        }

        *entries() {
            for (let i = 0; i < this.#entries.length; i++) {
                yield [this.#entries[i][0], this.#entries[i][1]];
            }
        }

        [Symbol.iterator]() {
            return this.entries();
        }

        forEach(callback) {
            if (typeof callback !== "function") {
                throw new makeTypeError("callback must be a function");
            }
            for (let i = 0; i < this.#entries.length; i++) {
                callback(this.#entries[i][1], this.#entries[i][0], this);
            }
        }

        toString() {
            const parts = [];
            for (let i = 0; i < this.#entries.length; i++) {
                const k = encodeURIComponent(this.#entries[i][0]);
                const v = encodeURIComponent(this.#entries[i][1]);
                parts.push(`${k}=${v}`);
            }
            return parts.join("&");
        }
    }

    function parseQueryString(rawQuery) {
        const result = unwrap(opParseQuery(rawQuery));
        return new QueryParams(result.params);
    }

    function splitUrlTarget(rawUrl) {
        const hashIdx = stringIndexOf(rawUrl, "#");
        const withoutHash = hashIdx >= 0 ? stringSlice(rawUrl, 0, hashIdx) : rawUrl;
        const qIdx = stringIndexOf(withoutHash, "?");
        if (qIdx >= 0) {
            return {
                path: stringSlice(withoutHash, 0, qIdx) || "/",
                rawQuery: stringSlice(withoutHash, qIdx + 1),
            };
        }
        return {
            path: withoutHash || "/",
            rawQuery: "",
        };
    }

    function parseCookies(header) {
        if (typeof header !== "string") {
            throw new makeTypeError("cookie header must be a string");
        }
        const res = unwrap(opParseCookies(header));
        return buildSafeRecord(res.cookies);
    }

    function formatSetCookie(cookie) {
        if (cookie === null || typeof cookie !== "object" || arrayIsArray(cookie)) {
            throw new makeTypeError("cookie options must be an object");
        }
        const res = unwrap(
            opFormatSetCookie(
                cookie.name,
                cookie.value,
                cookie.path,
                cookie.domain,
                cookie.maxAge,
                cookie.httpOnly,
                cookie.secure,
                cookie.sameSite,
            ),
        );
        return res.value;
    }

    function normalizeHeadersInput(headersInit) {
        const list = [];
        if (headersInit === undefined || headersInit === null) {
            return list;
        }
        if (arrayIsArray(headersInit)) {
            for (let i = 0; i < headersInit.length; i++) {
                const entry = headersInit[i];
                if (!arrayIsArray(entry) || entry.length !== 2) {
                    throw new makeTypeError("header entry must be a [name, value] tuple");
                }
                const name = entry[0];
                const value = entry[1];
                if (typeof name !== "string" || typeof value !== "string") {
                    throw new makeTypeError("header name and value must be strings");
                }
                validateHeader(name, value);
                list.push([stringToLowerCase(name), value]);
            }
            return list;
        }
        if (typeof headersInit === "object") {
            const entries = objectEntries(headersInit);
            for (let i = 0; i < entries.length; i++) {
                const name = entries[i][0];
                const value = entries[i][1];
                if (value === undefined) {
                    continue;
                }
                if (typeof value !== "string") {
                    throw new makeTypeError("header value must be a string");
                }
                validateHeader(name, value);
                list.push([stringToLowerCase(name), value]);
            }
            return list;
        }
        throw new makeTypeError("headers must be a plain object or [name, value][] array");
    }

    const RESPONSE_RAW_HEADERS = Symbol("neoHttpResponseRawHeaders");
    const RESPONSE_RAW_BODY = Symbol("neoHttpResponseRawBody");

    class HttpResponse {
        #status;
        #headersList;
        #headersRecord;
        #bodyBytes;

        constructor(status, headersList, bodyBytes) {
            this.#status = status;
            this.#headersList = headersList;
            this.#headersRecord = buildCollapsedHeadersRecord(headersList);
            this.#bodyBytes = bodyBytes;
            freeze(this);
        }

        get status() {
            return this.#status;
        }

        get headers() {
            return this.#headersRecord;
        }

        get body() {
            return copyBytes(this.#bodyBytes);
        }

        header(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            return hasOwn(this.#headersRecord, lower) ? this.#headersRecord[lower] : undefined;
        }

        headerValues(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            const out = [];
            for (let i = 0; i < this.#headersList.length; i++) {
                if (this.#headersList[i][0] === lower) {
                    out.push(this.#headersList[i][1]);
                }
            }
            return out;
        }

        bytes() {
            return copyBytes(this.#bodyBytes);
        }

        text() {
            return decodeUtf8(this.#bodyBytes);
        }

        json() {
            return jsonParse(this.text());
        }

        [RESPONSE_RAW_HEADERS]() {
            return this.#headersList;
        }

        [RESPONSE_RAW_BODY]() {
            return this.#bodyBytes;
        }
    }

    function cloneResponseWithExtraHeaders(res, extraPairs) {
        if (!(res instanceof HttpResponse)) {
            throw new makeTypeError("res must be an HttpResponse");
        }
        const merged = [...res[RESPONSE_RAW_HEADERS]()];
        for (let i = 0; i < extraPairs.length; i++) {
            const name = stringToLowerCase(extraPairs[i][0]);
            const value = extraPairs[i][1];
            validateHeader(name, value);
            merged.push([name, value]);
        }
        return new HttpResponse(res.status, merged, res[RESPONSE_RAW_BODY]());
    }

    function setHeaderIfAbsent(headersList, name, value) {
        const lower = stringToLowerCase(name);
        for (let i = 0; i < headersList.length; i++) {
            if (headersList[i][0] === lower) {
                return headersList;
            }
        }
        validateHeader(lower, value);
        return [...headersList, [lower, value]];
    }

    function replaceOrAppendHeader(headersList, name, value) {
        const lower = stringToLowerCase(name);
        validateHeader(lower, value);
        const next = [];
        let replaced = false;
        for (let i = 0; i < headersList.length; i++) {
            if (headersList[i][0] === lower) {
                if (!replaced) {
                    next.push([lower, value]);
                    replaced = true;
                }
            } else {
                next.push(headersList[i]);
            }
        }
        if (!replaced) {
            next.push([lower, value]);
        }
        return next;
    }

    function response(body, init) {
        if (init !== undefined && (init === null || typeof init !== "object" || arrayIsArray(init))) {
            throw new makeTypeError("ResponseInit must be an object");
        }
        const status = init && init.status !== undefined ? init.status : 200;
        if (!isInteger(status) || status < 100 || status > 599) {
            throw new makeRangeError("response status must be an integer between 100 and 599");
        }
        const headersList = normalizeHeadersInput(init ? init.headers : undefined);
        let bodyBytes;
        if (body === undefined || body === null) {
            bodyBytes = new makeUint8Array(0);
        } else if (typeof body === "string") {
            bodyBytes = encodeUtf8(body);
        } else if (body instanceof makeUint8Array) {
            bodyBytes = copyBytes(body);
        } else {
            throw new makeTypeError("response body must be a string, Uint8Array, null, or undefined");
        }
        return new HttpResponse(status, headersList, bodyBytes);
    }

    function responseWithDefaultContentType(body, contentType, init) {
        const res = response(body, init);
        const headersList = setHeaderIfAbsent(res[RESPONSE_RAW_HEADERS](), "content-type", contentType);
        return new HttpResponse(res.status, headersList, res[RESPONSE_RAW_BODY]());
    }

    function json(data, init) {
        const serialized = data === undefined ? "null" : jsonStringify(data);
        if (typeof serialized !== "string") {
            throw new makeTypeError("json data is not JSON-serializable");
        }
        return responseWithDefaultContentType(serialized, "application/json; charset=utf-8", init);
    }

    function html(body, init) {
        if (typeof body !== "string") {
            throw new makeTypeError("html body must be a string");
        }
        return responseWithDefaultContentType(body, "text/html; charset=utf-8", init);
    }

    function text(body, init) {
        if (typeof body !== "string") {
            throw new makeTypeError("text body must be a string");
        }
        return responseWithDefaultContentType(body, "text/plain; charset=utf-8", init);
    }

    function redirect(location, status = 302) {
        if (typeof location !== "string" || location.length === 0) {
            throw new makeTypeError("redirect location must be a non-empty string");
        }
        if (
            status !== 301 &&
            status !== 302 &&
            status !== 303 &&
            status !== 307 &&
            status !== 308
        ) {
            throw new makeRangeError("redirect status must be 301, 302, 303, 307, or 308");
        }
        validateHeader("location", location);
        return response(null, { status, headers: { location } });
    }

    function noContent(init) {
        if (init !== undefined && (init === null || typeof init !== "object" || arrayIsArray(init))) {
            throw new makeTypeError("ResponseInit must be an object");
        }
        return response(null, { headers: init ? init.headers : undefined, status: 204 });
    }

    function setCookie(res, cookie) {
        const formatted = formatSetCookie(cookie);
        return cloneResponseWithExtraHeaders(res, [["set-cookie", formatted]]);
    }

    class HttpRequest {
        #bodyBytes;
        #query;
        #cookies;

        constructor(spec) {
            this.method = spec.method;
            this.url = spec.url;
            this.path = spec.path;
            this.#query = spec.query;
            this.headers = spec.headersRecord;
            this.params = spec.paramsRecord;
            this.#cookies = spec.cookiesRecord;
            this.#bodyBytes = spec.bodyBytes;
            this.remoteAddr = spec.remoteAddr;
            freeze(this);
        }

        get query() {
            return this.#query;
        }

        get cookies() {
            return this.#cookies;
        }

        header(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("header name must be a string");
            }
            const lower = stringToLowerCase(name);
            return hasOwn(this.headers, lower) ? this.headers[lower] : undefined;
        }

        param(name) {
            if (typeof name !== "string") {
                throw new makeTypeError("param name must be a string");
            }
            return hasOwn(this.params, name) ? this.params[name] : undefined;
        }

        bodyBytes() {
            return copyBytes(this.#bodyBytes);
        }

        async text() {
            const res = opDecodeUtf8(this.#bodyBytes);
            if (!res || res.ok !== true) {
                throw badRequest("invalid UTF-8 request body", "NEO_HTTP_INVALID_BODY");
            }
            return res.text;
        }

        async json() {
            const bodyText = await this.text();
            try {
                return jsonParse(bodyText);
            } catch {
                throw badRequest("invalid JSON request body", "NEO_HTTP_INVALID_JSON");
            }
        }

        async form() {
            const bodyText = await this.text();
            return parseQueryString(bodyText);
        }
    }

    function buildHttpRequest(raw, paramsPairs) {
        const headersRecord = buildCollapsedHeadersRecord(raw.headers);
        const paramsRecord = buildSafeRecord(paramsPairs || []);
        let cookiesRecord = freeze({});
        if (hasOwn(headersRecord, "cookie")) {
            const parsed = opParseCookies(headersRecord.cookie);
            if (parsed && parsed.ok === true) {
                cookiesRecord = buildSafeRecord(parsed.cookies);
            }
        }
        const split = splitUrlTarget(raw.url);
        const query = parseQueryString(split.rawQuery);
        return new HttpRequest({
            method: raw.method,
            url: raw.url,
            path: raw.path || split.path,
            query,
            headersRecord,
            paramsRecord,
            cookiesRecord,
            bodyBytes: raw.body,
            remoteAddr: raw.remoteAddr,
        });
    }

    function errorToResponse(err, req) {
        const accept = req && typeof req.header === "function" ? req.header("accept") || "" : "";
        const wantsJson = stringIncludes(stringToLowerCase(accept), "application/json");
        const init = err.headers ? { status: err.status, headers: err.headers } : { status: err.status };
        if (wantsJson) {
            const payload = {
                error: err.message,
                status: err.status,
            };
            if (err.code !== undefined) {
                payload.code = err.code;
            }
            if (err.details !== undefined) {
                payload.details = err.details;
            }
            return json(payload, init);
        }
        return text(err.message, init);
    }

    function internalServerErrorResponse(req) {
        return errorToResponse(new HttpError(500, "Internal Server Error", "NEO_HTTP_INTERNAL"), req);
    }

    function compose(...middlewares) {
        for (let i = 0; i < middlewares.length; i++) {
            if (typeof middlewares[i] !== "function") {
                throw new makeTypeError("every middleware passed to compose must be a function");
            }
        }
        return function composedMiddleware(next) {
            if (typeof next !== "function") {
                throw new makeTypeError("next handler must be a function");
            }
            let current = next;
            for (let i = middlewares.length - 1; i >= 0; i--) {
                const mw = middlewares[i];
                const wrapped = mw(current);
                if (typeof wrapped !== "function") {
                    throw new makeTypeError("middleware must return a handler function");
                }
                current = wrapped;
            }
            return current;
        };
    }

    function recover(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("recover options must be an object");
        }
        const onError = options && options.onError;
        if (onError !== undefined && typeof onError !== "function") {
            throw new makeTypeError("recover options.onError must be a function");
        }
        return function recoverMiddleware(next) {
            return async function recoverHandler(req, ctx) {
                try {
                    return await next(req, ctx);
                } catch (err) {
                    if (err instanceof HttpError) {
                        return errorToResponse(err, req);
                    }
                    if (onError) {
                        try {
                            onError(err, req, ctx);
                        } catch {
                            // Never let an error observer break recovery.
                        }
                    }
                    return internalServerErrorResponse(req);
                }
            };
        };
    }

    function isSafeRequestId(value) {
        if (typeof value !== "string" || value.length === 0 || value.length > 128) {
            return false;
        }
        for (let i = 0; i < value.length; i++) {
            const code = value.charCodeAt(i);
            const ok =
                (code >= 48 && code <= 57) ||
                (code >= 65 && code <= 90) ||
                (code >= 97 && code <= 122) ||
                code === 45 ||
                code === 46 ||
                code === 95;
            if (!ok) {
                return false;
            }
        }
        return true;
    }

    function requestId(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("requestId options must be an object");
        }
        const headerName = options && options.headerName !== undefined ? options.headerName : "x-request-id";
        if (typeof headerName !== "string") {
            throw new makeTypeError("requestId options.headerName must be a string");
        }
        const lowerHeader = stringToLowerCase(headerName);
        validateHeader(lowerHeader, "id");

        const generator = options && options.generator;
        if (generator !== undefined && typeof generator !== "function") {
            throw new makeTypeError("requestId options.generator must be a function");
        }

        return function requestIdMiddleware(next) {
            return async function requestIdHandler(req, ctx) {
                const incoming = req.header(lowerHeader);
                let id = incoming !== undefined && isSafeRequestId(incoming) ? incoming : undefined;
                if (id === undefined) {
                    id = generator ? String(generator()) : unwrap(opRandomId()).id;
                    validateHeader(lowerHeader, id);
                }
                if (ctx && ctx.state && typeof ctx.state === "object") {
                    ctx.state.requestId = id;
                }
                const res = await next(req, ctx);
                if (!(res instanceof HttpResponse)) {
                    throw new makeTypeError("handler must return an HttpResponse");
                }
                const nextHeaders = replaceOrAppendHeader(res[RESPONSE_RAW_HEADERS](), lowerHeader, id);
                return new HttpResponse(res.status, nextHeaders, res[RESPONSE_RAW_BODY]());
            };
        };
    }

    function logger(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("logger options must be an object");
        }
        const sink = options && options.sink;
        if (sink !== undefined && typeof sink !== "function") {
            throw new makeTypeError("logger options.sink must be a function");
        }
        return function loggerMiddleware(next) {
            return async function loggerHandler(req, ctx) {
                const startedAt = Date.now();
                let status = 500;
                try {
                    const res = await next(req, ctx);
                    if (res instanceof HttpResponse) {
                        status = res.status;
                    }
                    return res;
                } catch (err) {
                    if (err instanceof HttpError) {
                        status = err.status;
                    }
                    throw err;
                } finally {
                    const durationMs = Math.max(0, Date.now() - startedAt);
                    const entry = freeze({
                        method: req.method,
                        path: req.path,
                        status,
                        durationMs,
                        requestId:
                            ctx && ctx.state && typeof ctx.state.requestId === "string"
                                ? ctx.state.requestId
                                : undefined,
                    });
                    if (sink) {
                        sink(entry);
                    } else {
                        const ridPart = entry.requestId ? ` requestId=${entry.requestId}` : "";
                        console.info(`${entry.method} ${entry.path} ${entry.status} ${entry.durationMs}ms${ridPart}`);
                    }
                }
            };
        };
    }

    function securityHeaders(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("securityHeaders options must be an object");
        }
        const defaults = [
            [
                "x-content-type-options",
                options && options.contentTypeOptions !== undefined
                    ? options.contentTypeOptions
                    : "nosniff",
            ],
            [
                "x-frame-options",
                options && options.frameOptions !== undefined ? options.frameOptions : "DENY",
            ],
            [
                "referrer-policy",
                options && options.referrerPolicy !== undefined
                    ? options.referrerPolicy
                    : "no-referrer",
            ],
        ];
        const enabled = [];
        for (let i = 0; i < defaults.length; i++) {
            const name = defaults[i][0];
            const val = defaults[i][1];
            if (val === false) {
                continue;
            }
            if (typeof val !== "string") {
                throw new makeTypeError(`securityHeaders ${name} value must be a string or false`);
            }
            validateHeader(name, val);
            enabled.push([name, val]);
        }

        return function securityHeadersMiddleware(next) {
            return async function securityHeadersHandler(req, ctx) {
                const res = await next(req, ctx);
                if (!(res instanceof HttpResponse)) {
                    throw new makeTypeError("handler must return an HttpResponse");
                }
                let nextHeaders = res[RESPONSE_RAW_HEADERS]();
                for (let i = 0; i < enabled.length; i++) {
                    nextHeaders = setHeaderIfAbsent(nextHeaders, enabled[i][0], enabled[i][1]);
                }
                return new HttpResponse(res.status, nextHeaders, res[RESPONSE_RAW_BODY]());
            };
        };
    }

    function cors(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("cors options must be an object");
        }
        const originOpt = options && options.origin !== undefined ? options.origin : "*";
        const methodsOpt =
            options && options.methods !== undefined
                ? options.methods
                : ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
        const allowHeadersOpt =
            options && options.allowHeaders !== undefined
                ? options.allowHeaders
                : ["Content-Type", "Authorization"];
        const exposeHeadersOpt = options && options.exposeHeaders !== undefined ? options.exposeHeaders : [];
        const credentials = options && options.credentials === true;
        const maxAge = options && options.maxAge !== undefined ? options.maxAge : 86400;

        if (!arrayIsArray(methodsOpt)) {
            throw new makeTypeError("cors options.methods must be an array of strings");
        }
        if (!arrayIsArray(allowHeadersOpt)) {
            throw new makeTypeError("cors options.allowHeaders must be an array of strings");
        }
        if (!arrayIsArray(exposeHeadersOpt)) {
            throw new makeTypeError("cors options.exposeHeaders must be an array of strings");
        }
        if (!isInteger(maxAge) || maxAge < 0) {
            throw new makeRangeError("cors options.maxAge must be a non-negative integer");
        }

        const allowMethodsStr = methodsOpt.join(", ");
        const allowHeadersStr = allowHeadersOpt.join(", ");
        const exposeHeadersStr = exposeHeadersOpt.join(", ");
        if (allowMethodsStr) validateHeader("access-control-allow-methods", allowMethodsStr);
        if (allowHeadersStr) validateHeader("access-control-allow-headers", allowHeadersStr);
        if (exposeHeadersStr) validateHeader("access-control-expose-headers", exposeHeadersStr);

        function resolveAllowedOrigin(reqOrigin, req) {
            if (typeof originOpt === "string") {
                if (originOpt === "*") {
                    if (credentials && reqOrigin) {
                        return reqOrigin;
                    }
                    return "*";
                }
                return reqOrigin === originOpt ? originOpt : null;
            }
            if (arrayIsArray(originOpt)) {
                if (!reqOrigin) return null;
                for (let i = 0; i < originOpt.length; i++) {
                    if (originOpt[i] === reqOrigin) {
                        return reqOrigin;
                    }
                }
                return null;
            }
            if (typeof originOpt === "function") {
                if (!reqOrigin) return null;
                const decision = originOpt(reqOrigin, req);
                if (decision === true) return reqOrigin;
                if (typeof decision === "string" && decision.length > 0) return decision;
                return null;
            }
            throw new makeTypeError("cors options.origin must be a string, string[], or function");
        }

        return function corsMiddleware(next) {
            return async function corsHandler(req, ctx) {
                const reqOrigin = req.header("origin");
                const allowedOrigin = resolveAllowedOrigin(reqOrigin, req);

                if (req.method === "OPTIONS" && (req.header("access-control-request-method") || reqOrigin)) {
                    const preflightHeaders = [];
                    if (allowedOrigin !== null) {
                        validateHeader("access-control-allow-origin", allowedOrigin);
                        preflightHeaders.push(["access-control-allow-origin", allowedOrigin]);
                        if (allowedOrigin !== "*") {
                            preflightHeaders.push(["vary", "Origin"]);
                        }
                    }
                    if (credentials && allowedOrigin !== null) {
                        preflightHeaders.push(["access-control-allow-credentials", "true"]);
                    }
                    if (allowMethodsStr) {
                        preflightHeaders.push(["access-control-allow-methods", allowMethodsStr]);
                    }
                    if (allowHeadersStr) {
                        preflightHeaders.push(["access-control-allow-headers", allowHeadersStr]);
                    }
                    preflightHeaders.push(["access-control-max-age", String(maxAge)]);
                    return new HttpResponse(204, preflightHeaders, new makeUint8Array(0));
                }

                const res = await next(req, ctx);
                if (!(res instanceof HttpResponse)) {
                    throw new makeTypeError("handler must return an HttpResponse");
                }
                let nextHeaders = res[RESPONSE_RAW_HEADERS]();
                if (allowedOrigin !== null) {
                    validateHeader("access-control-allow-origin", allowedOrigin);
                    nextHeaders = replaceOrAppendHeader(nextHeaders, "access-control-allow-origin", allowedOrigin);
                    if (allowedOrigin !== "*") {
                        nextHeaders = setHeaderIfAbsent(nextHeaders, "vary", "Origin");
                    }
                }
                if (credentials && allowedOrigin !== null) {
                    nextHeaders = replaceOrAppendHeader(nextHeaders, "access-control-allow-credentials", "true");
                }
                if (exposeHeadersStr) {
                    nextHeaders = replaceOrAppendHeader(nextHeaders, "access-control-expose-headers", exposeHeadersStr);
                }
                return new HttpResponse(res.status, nextHeaders, res[RESPONSE_RAW_BODY]());
            };
        };
    }

    function normalizeGroupPrefix(prefix) {
        if (typeof prefix !== "string" || !stringStartsWith(prefix, "/")) {
            throw new makeTypeError("group prefix must be a string starting with '/'");
        }
        if (stringIncludes(prefix, " ") || stringIncludes(prefix, "?") || stringIncludes(prefix, "#")) {
            throw new makeTypeError("group prefix must not contain whitespace, '?', or '#'");
        }
        if (prefix.length > 1 && stringEndsWith(prefix, "/")) {
            return stringSlice(prefix, 0, prefix.length - 1);
        }
        return prefix;
    }

    function joinPrefixedPattern(prefix, pattern) {
        if (typeof pattern !== "string" || pattern.length === 0) {
            throw new makeTypeError("route pattern must be a non-empty string");
        }
        if (!prefix || prefix === "/") {
            return pattern;
        }
        let methodPrefix = "";
        let rawPath = pattern;
        if (!stringStartsWith(pattern, "/")) {
            const spaceIdx = stringIndexOf(pattern, " ");
            if (spaceIdx <= 0) {
                return pattern;
            }
            methodPrefix = stringSlice(pattern, 0, spaceIdx + 1);
            rawPath = stringSlice(pattern, spaceIdx + 1);
        }
        if (!stringStartsWith(rawPath, "/")) {
            return pattern;
        }
        if (rawPath === "/") {
            return methodPrefix + prefix;
        }
        return methodPrefix + prefix + rawPath;
    }

    const MUX_DISPATCH = Symbol("neoHttpMuxDispatch");

    class ServeMux {
        #core;
        #prefix;
        #localMiddlewares;
        #pendingRoutes;
        #sealed;

        constructor(core, prefix = "", localMiddlewares = [], pendingRoutes = null) {
            if (core === undefined) {
                const created = unwrap(opRouterCreate());
                this.#core = {
                    routerRid: created.rid,
                    handlers: [],
                    rootMiddlewares: [],
                    composedDispatch: null,
                };
                this.#prefix = "";
                this.#localMiddlewares = [];
                this.#pendingRoutes = null;
                this.#sealed = false;
            } else {
                this.#core = core;
                this.#prefix = prefix;
                this.#localMiddlewares = localMiddlewares;
                this.#pendingRoutes = pendingRoutes;
                this.#sealed = false;
            }
        }

        handle(pattern, handler) {
            if (typeof pattern !== "string") {
                throw new makeTypeError("route pattern must be a string");
            }
            if (typeof handler !== "function") {
                throw new makeTypeError("route handler must be a function");
            }
            const fullPattern = joinPrefixedPattern(this.#prefix, pattern);
            if (this.#pendingRoutes !== null) {
                if (this.#sealed) {
                    throw new makeError("cannot add routes to a closed group");
                }
                this.#pendingRoutes.push({
                    pattern: fullPattern,
                    handler,
                    middlewares: this.#localMiddlewares,
                });
                return this;
            }
            const handlerId = this.#core.handlers.length;
            this.#core.handlers.push(handler);
            unwrap(opRouterAdd(this.#core.routerRid, fullPattern, handlerId));
            return this;
        }

        #addMethod(method, path, handler) {
            if (typeof path !== "string" || !stringStartsWith(path, "/")) {
                throw new makeTypeError("route path must be a string starting with '/'");
            }
            if (stringIncludes(path, " ")) {
                throw new makeTypeError("route path must not contain spaces");
            }
            return this.handle(`${method} ${path}`, handler);
        }

        get(path, handler) {
            return this.#addMethod("GET", path, handler);
        }

        post(path, handler) {
            return this.#addMethod("POST", path, handler);
        }

        put(path, handler) {
            return this.#addMethod("PUT", path, handler);
        }

        patch(path, handler) {
            return this.#addMethod("PATCH", path, handler);
        }

        delete(path, handler) {
            return this.#addMethod("DELETE", path, handler);
        }

        use(...middlewares) {
            for (let i = 0; i < middlewares.length; i++) {
                if (typeof middlewares[i] !== "function") {
                    throw new makeTypeError("middleware must be a function");
                }
            }
            if (this.#pendingRoutes !== null) {
                if (this.#sealed) {
                    throw new makeError("cannot add middleware to a closed group");
                }
                for (let i = 0; i < middlewares.length; i++) {
                    this.#localMiddlewares.push(middlewares[i]);
                }
            } else {
                for (let i = 0; i < middlewares.length; i++) {
                    this.#core.rootMiddlewares.push(middlewares[i]);
                }
                this.#core.composedDispatch = null;
            }
            return this;
        }

        group(prefix, fn) {
            if (typeof fn !== "function") {
                throw new makeTypeError("group callback must be a function");
            }
            const norm = normalizeGroupPrefix(prefix);
            const nextPrefix =
                this.#prefix === "" || this.#prefix === "/"
                    ? norm
                    : norm === "/"
                      ? this.#prefix
                      : this.#prefix + norm;
            const localCopy = [...this.#localMiddlewares];
            const pending = [];
            const sub = new ServeMux(this.#core, nextPrefix, localCopy, pending);
            fn(sub);
            sub.#sealed = true;

            for (let i = 0; i < pending.length; i++) {
                const entry = pending[i];
                const wrapped =
                    entry.middlewares.length > 0
                        ? compose(...entry.middlewares)(entry.handler)
                        : entry.handler;
                if (this.#pendingRoutes !== null) {
                    this.#pendingRoutes.push({
                        pattern: entry.pattern,
                        handler: wrapped,
                        middlewares: [],
                    });
                } else {
                    const handlerId = this.#core.handlers.length;
                    this.#core.handlers.push(wrapped);
                    unwrap(opRouterAdd(this.#core.routerRid, entry.pattern, handlerId));
                }
            }
            return this;
        }

        static(urlPrefix, rootDir, options) {
            if (typeof urlPrefix !== "string" || !stringStartsWith(urlPrefix, "/")) {
                throw new makeTypeError("static urlPrefix must be a string starting with '/'");
            }
            if (typeof rootDir !== "string" || rootDir.length === 0) {
                throw new makeTypeError("static rootDir must be a non-empty string");
            }
            if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
                throw new makeTypeError("static options must be an object");
            }
            let indexFile = "index.html";
            let hasIndex = true;
            if (options && options.index !== undefined) {
                if (options.index === null) {
                    indexFile = "";
                    hasIndex = false;
                } else if (typeof options.index === "string") {
                    indexFile = options.index;
                    hasIndex = true;
                } else {
                    throw new makeTypeError("static options.index must be a string or null");
                }
            }

            let normPrefix = urlPrefix;
            if (normPrefix.length > 1 && stringEndsWith(normPrefix, "/")) {
                normPrefix = stringSlice(normPrefix, 0, normPrefix.length - 1);
            }

            function serveStatic(relPath, req) {
                const headOnly = req.method === "HEAD";
                const resolved = opStaticResolve(rootDir, relPath, indexFile, hasIndex, headOnly);
                if (!resolved || resolved.ok !== true) {
                    const status = resolved && resolved.status ? resolved.status : 404;
                    const msg = resolved && resolved.message ? resolved.message : "Not Found";
                    throw new HttpError(status, msg, resolved ? resolved.code : undefined);
                }
                return response(resolved.body, {
                    status: 200,
                    headers: {
                        "content-type": resolved.mimeType,
                        "content-length": String(resolved.size),
                    },
                });
            }

            if (normPrefix === "/") {
                this.handle("GET /", (req) => serveStatic("", req));
                this.handle("GET /{path...}", (req) => serveStatic(req.param("path") || "", req));
            } else {
                this.handle(`GET ${normPrefix}`, (req) => serveStatic("", req));
                this.handle(`GET ${normPrefix}/{path...}`, (req) => serveStatic(req.param("path") || "", req));
            }
            return this;
        }

        async #dispatchCore(req, ctx) {
            const matched = opRouterMatch(this.#core.routerRid, req.method, req.path);
            if (matched.status === 404) {
                return errorToResponse(notFound(), req);
            }
            if (matched.status === 405) {
                return errorToResponse(methodNotAllowed(matched.allow), req);
            }
            const paramsRecord = buildSafeRecord(matched.params || []);
            const boundReq = new HttpRequest({
                method: req.method,
                url: req.url,
                path: req.path,
                query: req.query,
                headersRecord: req.headers,
                paramsRecord,
                cookiesRecord: req.cookies,
                bodyBytes: req.bodyBytes(),
                remoteAddr: req.remoteAddr,
            });
            const boundCtx = {
                params: paramsRecord,
                state: ctx.state,
                remoteAddr: req.remoteAddr,
            };
            const handler = this.#core.handlers[matched.handlerId];
            try {
                const result = await handler(boundReq, boundCtx);
                if (!(result instanceof HttpResponse)) {
                    throw new makeTypeError("route handler must return an HttpResponse");
                }
                return result;
            } catch (err) {
                if (err instanceof HttpError) {
                    return errorToResponse(err, boundReq);
                }
                throw err;
            }
        }

        [MUX_DISPATCH]() {
            if (!this.#core.composedDispatch) {
                const base = (req, ctx) => this.#dispatchCore(req, ctx);
                this.#core.composedDispatch =
                    this.#core.rootMiddlewares.length > 0
                        ? compose(...this.#core.rootMiddlewares)(base)
                        : base;
            }
            return this.#core.composedDispatch;
        }

        async fetch(input, init) {
            let url;
            let method = "GET";
            let headersInit;
            let bodyInit;
            let remoteAddr = "127.0.0.1:0";

            if (typeof input === "string") {
                url = input;
            } else if (input !== null && typeof input === "object" && !arrayIsArray(input)) {
                if (typeof input.url !== "string" || input.url.length === 0) {
                    throw new makeTypeError("FetchInput.url must be a non-empty string");
                }
                url = input.url;
                if (input.method !== undefined) method = input.method;
                if (input.headers !== undefined) headersInit = input.headers;
                if (input.body !== undefined) bodyInit = input.body;
                if (input.remoteAddr !== undefined) remoteAddr = input.remoteAddr;
            } else {
                throw new makeTypeError("fetch input must be a URL path string or FetchInput object");
            }

            if (init !== undefined) {
                if (init === null || typeof init !== "object" || arrayIsArray(init)) {
                    throw new makeTypeError("FetchInit must be an object");
                }
                if (init.method !== undefined) method = init.method;
                if (init.headers !== undefined) headersInit = init.headers;
                if (init.body !== undefined) bodyInit = init.body;
                if (init.remoteAddr !== undefined) remoteAddr = init.remoteAddr;
            }

            if (typeof method !== "string" || method.length === 0) {
                throw new makeTypeError("request method must be a non-empty string");
            }
            if (!stringStartsWith(url, "/")) {
                throw new makeTypeError("fetch input URL path must start with '/'");
            }
            if (typeof remoteAddr !== "string") {
                throw new makeTypeError("remoteAddr must be a string");
            }

            const headersList = normalizeHeadersInput(headersInit);
            let bodyBytes;
            if (bodyInit === undefined || bodyInit === null) {
                bodyBytes = new makeUint8Array(0);
            } else if (typeof bodyInit === "string") {
                bodyBytes = encodeUtf8(bodyInit);
            } else if (bodyInit instanceof makeUint8Array) {
                bodyBytes = copyBytes(bodyInit);
            } else {
                throw new makeTypeError("request body must be a string, Uint8Array, null, or undefined");
            }

            const split = splitUrlTarget(url);
            const req = buildHttpRequest(
                {
                    method: stringToUpperCase(stringTrim(method)),
                    url,
                    path: split.path,
                    headers: headersList,
                    body: bodyBytes,
                    remoteAddr,
                },
                [],
            );
            const ctx = {
                params: req.params,
                state: objectCreate(null),
                remoteAddr: req.remoteAddr,
            };
            const dispatch = this[MUX_DISPATCH]();
            try {
                const res = await dispatch(req, ctx);
                if (!(res instanceof HttpResponse)) {
                    throw new makeTypeError("handler must return an HttpResponse");
                }
                return res;
            } catch (err) {
                if (err instanceof HttpError) {
                    return errorToResponse(err, req);
                }
                throw err;
            }
        }
    }

    function createRouter() {
        return new ServeMux();
    }

    function serve(optionsOrHandler, maybeHandler) {
        let options;
        let handlerOrMux;
        if (maybeHandler === undefined && (typeof optionsOrHandler === "function" || optionsOrHandler instanceof ServeMux)) {
            options = {};
            handlerOrMux = optionsOrHandler;
        } else {
            options = optionsOrHandler || {};
            handlerOrMux = maybeHandler;
        }

        if (options === null || typeof options !== "object" || arrayIsArray(options)) {
            throw new makeTypeError("serve options must be an object");
        }
        const host = options.host !== undefined ? options.host : "127.0.0.1";
        const port = options.port !== undefined ? options.port : 8080;
        const maxBodyBytes =
            options.maxBodyBytes !== undefined ? options.maxBodyBytes : DEFAULT_MAX_BODY_BYTES;
        const signal = options.signal;

        if (signal !== undefined) {
            if (signal === null || typeof signal !== "object" || typeof signal.addEventListener !== "function") {
                throw new makeTypeError("options.signal must be an AbortSignal-like object");
            }
        }

        let dispatchFn;
        if (typeof handlerOrMux === "function") {
            dispatchFn = handlerOrMux;
        } else if (handlerOrMux instanceof ServeMux) {
            dispatchFn = handlerOrMux[MUX_DISPATCH]();
        } else {
            throw new makeTypeError("serve requires a Handler function or ServeMux instance");
        }

        const bound = unwrap(opListen(host, port, maxBodyBytes));
        const serverRid = bound.serverRid;
        let closed = false;
        let closingPromise = null;
        const inFlight = new Set();

        function detachSignal() {
            if (signal && typeof signal.removeEventListener === "function") {
                signal.removeEventListener("abort", onAbort);
            }
        }

        function onAbort() {
            void close();
        }

        async function dispatchOne(reqRid) {
            let req;
            try {
                const raw = unwrap(opTakeRequest(reqRid));
                req = buildHttpRequest(raw, []);
                const ctx = {
                    params: req.params,
                    state: objectCreate(null),
                    remoteAddr: req.remoteAddr,
                };
                const res = await dispatchFn(req, ctx);
                if (!(res instanceof HttpResponse)) {
                    throw new makeTypeError("handler must return an HttpResponse");
                }
                opRespond(reqRid, res.status, res[RESPONSE_RAW_HEADERS](), res[RESPONSE_RAW_BODY]());
            } catch (err) {
                const fallback =
                    err instanceof HttpError
                        ? errorToResponse(err, req)
                        : internalServerErrorResponse(req);
                opRespond(
                    reqRid,
                    fallback.status,
                    fallback[RESPONSE_RAW_HEADERS](),
                    fallback[RESPONSE_RAW_BODY](),
                );
            }
        }

        const finished = (async () => {
            try {
                while (!closed) {
                    const reqRid = await opNextRequest(serverRid);
                    if (reqRid < 0) {
                        break;
                    }
                    const task = dispatchOne(reqRid);
                    inFlight.add(task);
                    task.finally(() => {
                        inFlight.delete(task);
                    });
                }
            } finally {
                if (inFlight.size > 0) {
                    await promiseAllSettled(Array.from(inFlight));
                }
                detachSignal();
            }
        })();

        function close() {
            if (closingPromise) {
                return closingPromise;
            }
            closed = true;
            detachSignal();
            closingPromise = (async () => {
                await opClose(serverRid);
                await finished;
            })();
            return closingPromise;
        }

        if (signal) {
            if (signal.aborted) {
                void close();
            } else {
                signal.addEventListener("abort", onAbort, { once: true });
            }
        }

        const server = {
            host: bound.host,
            port: bound.port,
            addr: bound.addr,
            url: bound.url,
            close,
            finished,
        };
        if (typeof Symbol === "function" && Symbol.asyncDispose) {
            defineProperty(server, Symbol.asyncDispose, {
                value: close,
                enumerable: false,
                configurable: true,
                writable: false,
            });
        }
        return freeze(server);
    }

    return freeze({
        HttpError,
        HttpRequest,
        HttpResponse,
        ServeMux,
        createRouter,
        response,
        json,
        html,
        text,
        redirect,
        noContent,
        setCookie,
        httpError,
        badRequest,
        unauthorized,
        forbidden,
        notFound,
        conflict,
        methodNotAllowed,
        recover,
        requestId,
        logger,
        securityHeaders,
        cors,
        compose,
        serve,
        parseCookies,
        formatSetCookie,
    });
})
