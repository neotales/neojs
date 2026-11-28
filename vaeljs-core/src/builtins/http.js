// The `vael:http` built-in module factory, evaluated inside `vael.js` only when
// the `vael-http` Cargo feature is enabled.
(function createVaelHttp(deps) {
    "use strict";

    const ops = deps.ops;
    const opDefaultMaxBodyBytes = ops.op_vael_http_default_max_body_bytes;
    const opRandomId = ops.op_vael_http_random_id;
    const opRouterCreate = ops.op_vael_http_router_create;
    const opRouterAdd = ops.op_vael_http_router_add;
    const opRouterMatch = ops.op_vael_http_router_match;
    const opStaticResolve = ops.op_vael_http_static_resolve;
    const opParseCookies = ops.op_vael_http_parse_cookies;
    const opFormatSetCookie = ops.op_vael_http_format_set_cookie;
    const opValidateHeader = ops.op_vael_http_validate_header;
    const opEncodeUtf8 = ops.op_vael_http_encode_utf8;
    const opDecodeUtf8 = ops.op_vael_http_decode_utf8;
    const opParseQuery = ops.op_vael_http_parse_query;
    const opListen = ops.op_vael_http_listen;
    const opNextRequest = ops.op_vael_http_next_request;
    const opTakeRequest = ops.op_vael_http_take_request;
    const opRespond = ops.op_vael_http_respond;
    const opClose = ops.op_vael_http_close;

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const makeString = String;
    const makeUint8Array = Uint8Array;
    const freeze = Object.freeze;
    const defineProperty = Object.defineProperty;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const hasOwn = Object.hasOwn;
    const objectEntries = Object.entries;
    const objectCreate = Object.create;
    const arrayIsArray = Array.isArray;
    const arrayFrom = Array.from;
    const isInteger = Number.isInteger;
    const promiseAllSettled = Promise.allSettled.bind(Promise);
    const promiseFinally = Function.prototype.call.bind(Promise.prototype.finally);
    const jsonStringify = JSON.stringify;
    const jsonParse = JSON.parse;
    const stringToLowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    const stringToUpperCase = Function.prototype.call.bind(String.prototype.toUpperCase);
    const stringTrim = Function.prototype.call.bind(String.prototype.trim);
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const stringSplit = Function.prototype.call.bind(String.prototype.split);
    const stringIndexOf = Function.prototype.call.bind(String.prototype.indexOf);
    const stringIncludes = Function.prototype.call.bind(String.prototype.includes);
    const stringStartsWith = Function.prototype.call.bind(String.prototype.startsWith);
    const stringEndsWith = Function.prototype.call.bind(String.prototype.endsWith);
    const stringCharCodeAt = Function.prototype.call.bind(String.prototype.charCodeAt);
    const numberToString = Function.prototype.call.bind(Number.prototype.toString);

    const typedArrayProto = getPrototypeOf(Uint8Array.prototype);
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);
    const typedArrayIndexOf = Function.prototype.call.bind(typedArrayProto.indexOf);
    const typedArrayToStringTag = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayProto, Symbol.toStringTag).get,
    );
    const typedArrayByteLength = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayProto, "byteLength").get,
    );

    const makeSet = Set;
    const setAdd = Function.prototype.call.bind(Set.prototype.add);
    const setDelete = Function.prototype.call.bind(Set.prototype.delete);
    const setSize = Function.prototype.call.bind(
        getOwnPropertyDescriptor(Set.prototype, "size").get,
    );
    const setValues = Function.prototype.call.bind(Set.prototype.values);

    const makeWeakMap = WeakMap;
    const weakMapGet = Function.prototype.call.bind(WeakMap.prototype.get);
    const weakMapSet = Function.prototype.call.bind(WeakMap.prototype.set);
    const weakMapHas = Function.prototype.call.bind(WeakMap.prototype.has);

    const responseState = new makeWeakMap();
    const muxState = new makeWeakMap();

    const DEFAULT_MAX_BODY_BYTES = opDefaultMaxBodyBytes();

    function ownField(obj, key) {
        return hasOwn(obj, key) ? obj[key] : undefined;
    }

    function defineOwn(target, key, value, enumerable = true) {
        defineProperty(target, key, {
            __proto__: null,
            value,
            writable: true,
            enumerable,
            configurable: true,
        });
    }

    function assertGenuineUint8Array(value, message) {
        let tag;
        try {
            tag = typedArrayToStringTag(value);
        } catch {
            throw new makeTypeError(message);
        }
        if (tag !== "Uint8Array") {
            throw new makeTypeError(message);
        }
        // Validates attached & in-bounds (including ResizableArrayBuffer views) in O(1).
        typedArrayIndexOf(value, 0, 1 / 0);
    }

    function copyBytes(bytes) {
        assertGenuineUint8Array(bytes, "value must be a Uint8Array");
        const len = typedArrayByteLength(bytes);
        const copy = new makeUint8Array(len);
        typedArraySet(copy, bytes);
        return copy;
    }

    function throwFailure(result) {
        let err;
        if (result.name === "TypeError") {
            err = new makeTypeError(result.message);
        } else if (result.name === "RangeError") {
            err = new makeRangeError(result.message);
        } else {
            err = new makeError(result.message);
            if (result.name) {
                defineOwn(err, "name", result.name, false);
            }
        }
        if (result.code !== undefined) {
            defineOwn(err, "code", result.code, true);
        }
        throw err;
    }

    function unwrap(result) {
        if (!result || result.ok !== true) {
            throwFailure(result || { name: "Error", message: "HTTP host operation failed" });
        }
        return result;
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
        const record = objectCreate(null);
        for (let i = 0; i < pairs.length; i++) {
            const pair = pairs[i];
            defineOwn(record, pair[0], pair[1], true);
        }
        return freeze(record);
    }

    function buildCollapsedHeadersRecord(pairs) {
        const record = objectCreate(null);
        for (let i = 0; i < pairs.length; i++) {
            const name = stringToLowerCase(pairs[i][0]);
            const val = pairs[i][1];
            if (hasOwn(record, name)) {
                const sep = name === "cookie" ? "; " : ", ";
                defineOwn(record, name, record[name] + sep + val, true);
            } else {
                defineOwn(record, name, val, true);
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
            defineOwn(this, "name", "HttpError", false);
            defineOwn(this, "status", status, true);
            if (code !== undefined) {
                if (typeof code !== "string") {
                    throw new makeTypeError("HttpError code must be a string");
                }
                defineOwn(this, "code", code, true);
            }
            if (details !== undefined) {
                defineOwn(this, "details", details, true);
            }
        }
    }

    function httpError(status, message, code, details) {
        return new HttpError(status, message, code, details);
    }

    function badRequest(message = "Bad Request", code = "VAEL_HTTP_BAD_REQUEST") {
        return new HttpError(400, message, code);
    }

    function unauthorized(message = "Unauthorized", code = "VAEL_HTTP_UNAUTHORIZED") {
        return new HttpError(401, message, code);
    }

    function forbidden(message = "Forbidden", code = "VAEL_HTTP_FORBIDDEN") {
        return new HttpError(403, message, code);
    }

    function notFound(message = "Not Found", code = "VAEL_HTTP_NOT_FOUND") {
        return new HttpError(404, message, code);
    }

    function conflict(message = "Conflict", code = "VAEL_HTTP_CONFLICT") {
        return new HttpError(409, message, code);
    }

    function methodNotAllowed(allow, message = "Method Not Allowed") {
        const err = new HttpError(405, message, "VAEL_HTTP_METHOD_NOT_ALLOWED");
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
                const allowObj = objectCreate(null);
                defineOwn(allowObj, "allow", allowHeader, true);
                defineOwn(err, "headers", freeze(allowObj), true);
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
                ownField(cookie, "name"),
                ownField(cookie, "value"),
                ownField(cookie, "path"),
                ownField(cookie, "domain"),
                ownField(cookie, "maxAge"),
                ownField(cookie, "httpOnly"),
                ownField(cookie, "secure"),
                ownField(cookie, "sameSite"),
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
            weakMapSet(responseState, this, {
                headersList,
                bodyBytes,
            });
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
    }

    function getResponseInternal(res) {
        const state = weakMapGet(responseState, res);
        if (!state) {
            throw new makeTypeError("handler must return an HttpResponse");
        }
        return state;
    }

    function cloneResponseWithExtraHeaders(res, extraPairs) {
        const state = weakMapGet(responseState, res);
        if (!state) {
            throw new makeTypeError("res must be an HttpResponse");
        }
        const merged = [...state.headersList];
        for (let i = 0; i < extraPairs.length; i++) {
            const name = stringToLowerCase(extraPairs[i][0]);
            const value = extraPairs[i][1];
            validateHeader(name, value);
            merged.push([name, value]);
        }
        return new HttpResponse(res.status, merged, state.bodyBytes);
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

    function ensureVaryOrigin(headersList) {
        const next = [];
        let found = false;
        for (let i = 0; i < headersList.length; i++) {
            const name = headersList[i][0];
            const val = headersList[i][1];
            if (name === "vary") {
                found = true;
                const tokens = stringSplit(val, ",");
                let hasOrigin = false;
                for (let j = 0; j < tokens.length; j++) {
                    const tok = stringToLowerCase(stringTrim(tokens[j]));
                    if (tok === "origin" || tok === "*") {
                        hasOrigin = true;
                        break;
                    }
                }
                next.push([name, hasOrigin ? val : `${val}, Origin`]);
            } else {
                next.push(headersList[i]);
            }
        }
        if (!found) {
            next.push(["vary", "Origin"]);
        }
        return next;
    }

    function response(body, init) {
        if (init !== undefined && (init === null || typeof init !== "object" || arrayIsArray(init))) {
            throw new makeTypeError("ResponseInit must be an object");
        }
        const statusOpt = init ? ownField(init, "status") : undefined;
        const status = statusOpt !== undefined ? statusOpt : 200;
        if (!isInteger(status) || status < 100 || status > 599) {
            throw new makeRangeError("response status must be an integer between 100 and 599");
        }
        const headersList = normalizeHeadersInput(init ? ownField(init, "headers") : undefined);
        let bodyBytes;
        if (body === undefined || body === null) {
            bodyBytes = new makeUint8Array(0);
        } else if (typeof body === "string") {
            bodyBytes = encodeUtf8(body);
        } else {
            bodyBytes = copyBytes(body);
        }
        return new HttpResponse(status, headersList, bodyBytes);
    }

    function responseWithDefaultContentType(body, contentType, init) {
        const res = response(body, init);
        const state = getResponseInternal(res);
        const headersList = setHeaderIfAbsent(state.headersList, "content-type", contentType);
        return new HttpResponse(res.status, headersList, state.bodyBytes);
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
        return response(null, {
            headers: init ? ownField(init, "headers") : undefined,
            status: 204,
        });
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
                throw badRequest("invalid UTF-8 request body", "VAEL_HTTP_INVALID_BODY");
            }
            return res.text;
        }

        async json() {
            const bodyText = await this.text();
            try {
                return jsonParse(bodyText);
            } catch {
                throw badRequest("invalid JSON request body", "VAEL_HTTP_INVALID_JSON");
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
        let cookiesRecord = freeze(objectCreate(null));
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
        const rawStatus = ownField(err, "status");
        const status = isInteger(rawStatus) && rawStatus >= 100 && rawStatus <= 599 ? rawStatus : 500;
        const msg = String(ownField(err, "message") ?? defaultStatusText(status));
        const code = ownField(err, "code");
        const details = ownField(err, "details");
        const errHeaders = ownField(err, "headers");

        const accept = req && typeof req.header === "function" ? req.header("accept") || "" : "";
        const wantsJson = stringIncludes(stringToLowerCase(accept), "application/json");
        const init = errHeaders ? { status, headers: errHeaders } : { status };
        if (wantsJson) {
            const payload = {
                error: msg,
                status,
            };
            if (code !== undefined) {
                payload.code = code;
            }
            if (details !== undefined) {
                payload.details = details;
            }
            return json(payload, init);
        }
        return text(msg, init);
    }

    function internalServerErrorResponse(req) {
        return errorToResponse(new HttpError(500, "Internal Server Error", "VAEL_HTTP_INTERNAL"), req);
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
        const onError = options ? ownField(options, "onError") : undefined;
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
        const headerNameOpt = options ? ownField(options, "headerName") : undefined;
        const headerName = headerNameOpt !== undefined ? headerNameOpt : "x-request-id";
        if (typeof headerName !== "string") {
            throw new makeTypeError("requestId options.headerName must be a string");
        }
        const lowerHeader = stringToLowerCase(headerName);
        validateHeader(lowerHeader, "id");

        const generator = options ? ownField(options, "generator") : undefined;
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
                const resState = getResponseInternal(res);
                const nextHeaders = replaceOrAppendHeader(resState.headersList, lowerHeader, id);
                return new HttpResponse(res.status, nextHeaders, resState.bodyBytes);
            };
        };
    }

    function sanitizeLogText(value) {
        const text = makeString(value);
        let out = "";
        let lastClean = 0;
        for (let i = 0; i < text.length; i++) {
            const code = stringCharCodeAt(text, i);
            if (code <= 0x1f || code === 0x7f || (code >= 0x80 && code <= 0x9f)) {
                if (i > lastClean) {
                    out += stringSlice(text, lastClean, i);
                }
                const hex = numberToString(code, 16);
                out += code < 0x10 ? `\\x0${hex}` : `\\x${hex}`;
                lastClean = i + 1;
            }
        }
        if (lastClean === 0) {
            return text;
        }
        if (lastClean < text.length) {
            out += stringSlice(text, lastClean);
        }
        return out;
    }

    function logger(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("logger options must be an object");
        }
        const sink = options ? ownField(options, "sink") : undefined;
        if (sink !== undefined && typeof sink !== "function") {
            throw new makeTypeError("logger options.sink must be a function");
        }
        return function loggerMiddleware(next) {
            return async function loggerHandler(req, ctx) {
                const startedAt = Date.now();
                let status = 500;
                try {
                    const res = await next(req, ctx);
                    if (weakMapHas(responseState, res)) {
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
                        const ridPart = entry.requestId ? ` requestId=${sanitizeLogText(entry.requestId)}` : "";
                        console.info(
                            `${sanitizeLogText(entry.method)} ${sanitizeLogText(entry.path)} ${entry.status} ${entry.durationMs}ms${ridPart}`,
                        );
                    }
                }
            };
        };
    }

    function securityHeaders(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("securityHeaders options must be an object");
        }
        const cto = options ? ownField(options, "contentTypeOptions") : undefined;
        const fo = options ? ownField(options, "frameOptions") : undefined;
        const rp = options ? ownField(options, "referrerPolicy") : undefined;
        const defaults = [
            ["x-content-type-options", cto !== undefined ? cto : "nosniff"],
            ["x-frame-options", fo !== undefined ? fo : "DENY"],
            ["referrer-policy", rp !== undefined ? rp : "no-referrer"],
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
                const resState = getResponseInternal(res);
                let nextHeaders = resState.headersList;
                for (let i = 0; i < enabled.length; i++) {
                    nextHeaders = setHeaderIfAbsent(nextHeaders, enabled[i][0], enabled[i][1]);
                }
                return new HttpResponse(res.status, nextHeaders, resState.bodyBytes);
            };
        };
    }

    function cors(options) {
        if (options !== undefined && (options === null || typeof options !== "object" || arrayIsArray(options))) {
            throw new makeTypeError("cors options must be an object");
        }
        const originRaw = options ? ownField(options, "origin") : undefined;
        const methodsRaw = options ? ownField(options, "methods") : undefined;
        const allowHeadersRaw = options ? ownField(options, "allowHeaders") : undefined;
        const exposeHeadersRaw = options ? ownField(options, "exposeHeaders") : undefined;
        const credentialsRaw = options ? ownField(options, "credentials") : undefined;
        const maxAgeRaw = options ? ownField(options, "maxAge") : undefined;

        const originOpt = originRaw !== undefined ? originRaw : "*";
        const methodsOpt =
            methodsRaw !== undefined
                ? methodsRaw
                : ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
        const allowHeadersOpt =
            allowHeadersRaw !== undefined ? allowHeadersRaw : ["Content-Type", "Authorization"];
        const exposeHeadersOpt = exposeHeadersRaw !== undefined ? exposeHeadersRaw : [];
        if (credentialsRaw !== undefined && typeof credentialsRaw !== "boolean") {
            throw new makeTypeError("cors options.credentials must be a boolean");
        }
        const credentials = credentialsRaw === true;
        const maxAge = maxAgeRaw !== undefined ? maxAgeRaw : 86400;

        if (originOpt === "*" && credentials) {
            throw new makeTypeError("cors cannot combine origin '*' with credentials: true");
        }
        if (
            typeof originOpt !== "string" &&
            !arrayIsArray(originOpt) &&
            typeof originOpt !== "function"
        ) {
            throw new makeTypeError("cors options.origin must be a string, string[], or function");
        }
        if (arrayIsArray(originOpt)) {
            for (let i = 0; i < originOpt.length; i++) {
                if (typeof originOpt[i] !== "string") {
                    throw new makeTypeError("cors options.origin array entries must be strings");
                }
            }
        }
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

        const dynamicOrigin = originOpt !== "*";

        function resolveAllowedOrigin(reqOrigin, req) {
            if (typeof originOpt === "string") {
                if (originOpt === "*") {
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
            if (!reqOrigin) return null;
            const decision = originOpt(reqOrigin, req);
            if (decision === true) return reqOrigin;
            if (typeof decision === "string" && decision.length > 0) return decision;
            return null;
        }

        return function corsMiddleware(next) {
            return async function corsHandler(req, ctx) {
                const reqOrigin = req.header("origin");
                const allowedOrigin = resolveAllowedOrigin(reqOrigin, req);

                if (req.method === "OPTIONS" && (req.header("access-control-request-method") || reqOrigin)) {
                    let preflightHeaders = [];
                    if (allowedOrigin !== null) {
                        validateHeader("access-control-allow-origin", allowedOrigin);
                        preflightHeaders.push(["access-control-allow-origin", allowedOrigin]);
                    }
                    if (dynamicOrigin || (allowedOrigin !== null && allowedOrigin !== "*")) {
                        preflightHeaders = ensureVaryOrigin(preflightHeaders);
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
                const resState = getResponseInternal(res);
                let nextHeaders = resState.headersList;
                if (allowedOrigin !== null) {
                    validateHeader("access-control-allow-origin", allowedOrigin);
                    nextHeaders = replaceOrAppendHeader(
                        nextHeaders,
                        "access-control-allow-origin",
                        allowedOrigin,
                    );
                }
                if (dynamicOrigin || (allowedOrigin !== null && allowedOrigin !== "*")) {
                    nextHeaders = ensureVaryOrigin(nextHeaders);
                }
                if (credentials && allowedOrigin !== null) {
                    nextHeaders = replaceOrAppendHeader(
                        nextHeaders,
                        "access-control-allow-credentials",
                        "true",
                    );
                }
                if (exposeHeadersStr) {
                    nextHeaders = replaceOrAppendHeader(
                        nextHeaders,
                        "access-control-expose-headers",
                        exposeHeadersStr,
                    );
                }
                return new HttpResponse(res.status, nextHeaders, resState.bodyBytes);
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
                weakMapSet(muxState, this, () => this.#getDispatch());
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
                });
                return this;
            }
            const handlerId = this.#core.handlers.length;
            unwrap(opRouterAdd(this.#core.routerRid, fullPattern, handlerId));
            this.#core.handlers.push(handler);
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

            // Snapshot group middlewares after `fn(sub)` returns so `.use(...)` in any order
            // within the group applies cleanly, and validate `opRouterAdd` before pushing handlers.
            const groupStack = [...localCopy];
            for (let i = 0; i < pending.length; i++) {
                const entry = pending[i];
                const wrapped =
                    groupStack.length > 0 ? compose(...groupStack)(entry.handler) : entry.handler;
                if (this.#pendingRoutes !== null) {
                    this.#pendingRoutes.push({
                        pattern: entry.pattern,
                        handler: wrapped,
                    });
                } else {
                    const handlerId = this.#core.handlers.length;
                    unwrap(opRouterAdd(this.#core.routerRid, entry.pattern, handlerId));
                    this.#core.handlers.push(wrapped);
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
            const indexOpt = options ? ownField(options, "index") : undefined;
            if (indexOpt !== undefined) {
                if (indexOpt === null) {
                    indexFile = "";
                    hasIndex = false;
                } else if (typeof indexOpt === "string") {
                    indexFile = indexOpt;
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
                getResponseInternal(result);
                return result;
            } catch (err) {
                if (err instanceof HttpError) {
                    return errorToResponse(err, boundReq);
                }
                throw err;
            }
        }

        #getDispatch() {
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
                const inputUrl = ownField(input, "url");
                if (typeof inputUrl !== "string" || inputUrl.length === 0) {
                    throw new makeTypeError("FetchInput.url must be a non-empty string");
                }
                url = inputUrl;
                const m = ownField(input, "method");
                const h = ownField(input, "headers");
                const b = ownField(input, "body");
                const r = ownField(input, "remoteAddr");
                if (m !== undefined) method = m;
                if (h !== undefined) headersInit = h;
                if (b !== undefined) bodyInit = b;
                if (r !== undefined) remoteAddr = r;
            } else {
                throw new makeTypeError("fetch input must be a URL path string or FetchInput object");
            }

            if (init !== undefined) {
                if (init === null || typeof init !== "object" || arrayIsArray(init)) {
                    throw new makeTypeError("FetchInit must be an object");
                }
                const m = ownField(init, "method");
                const h = ownField(init, "headers");
                const b = ownField(init, "body");
                const r = ownField(init, "remoteAddr");
                if (m !== undefined) method = m;
                if (h !== undefined) headersInit = h;
                if (b !== undefined) bodyInit = b;
                if (r !== undefined) remoteAddr = r;
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
            } else {
                bodyBytes = copyBytes(bodyInit);
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
            const dispatch = this.#getDispatch();
            try {
                const res = await dispatch(req, ctx);
                getResponseInternal(res);
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
        if (arguments.length === 1) {
            if (typeof optionsOrHandler === "function" || weakMapHas(muxState, optionsOrHandler)) {
                options = objectCreate(null);
                handlerOrMux = optionsOrHandler;
            } else {
                throw new makeTypeError("serve requires a Handler function or ServeMux instance");
            }
        } else {
            if (optionsOrHandler === undefined) {
                options = objectCreate(null);
            } else if (
                optionsOrHandler === null ||
                typeof optionsOrHandler !== "object" ||
                arrayIsArray(optionsOrHandler)
            ) {
                throw new makeTypeError("serve options must be an object");
            } else {
                options = optionsOrHandler;
            }
            handlerOrMux = maybeHandler;
        }

        const hostOpt = ownField(options, "host");
        const portOpt = ownField(options, "port");
        const maxBodyOpt = ownField(options, "maxBodyBytes");
        const signal = ownField(options, "signal");

        const host = hostOpt !== undefined ? hostOpt : "127.0.0.1";
        const port = portOpt !== undefined ? portOpt : 8080;
        const maxBodyBytes = maxBodyOpt !== undefined ? maxBodyOpt : DEFAULT_MAX_BODY_BYTES;

        if (signal !== undefined) {
            if (signal === null || typeof signal !== "object" || typeof signal.addEventListener !== "function") {
                throw new makeTypeError("options.signal must be an AbortSignal-like object");
            }
        }

        let dispatchFn;
        if (typeof handlerOrMux === "function") {
            dispatchFn = handlerOrMux;
        } else if (weakMapHas(muxState, handlerOrMux)) {
            dispatchFn = weakMapGet(muxState, handlerOrMux)();
        } else {
            throw new makeTypeError("serve requires a Handler function or ServeMux instance");
        }

        const bound = unwrap(opListen(host, port, maxBodyBytes));
        const serverRid = bound.serverRid;
        let closed = false;
        let closingPromise = null;
        let closeOpPromise = null;
        const inFlight = new makeSet();

        function detachSignal() {
            if (signal && typeof signal.removeEventListener === "function") {
                try {
                    signal.removeEventListener("abort", onAbort);
                } catch {
                    // Best-effort cleanup on hostile signal objects.
                }
            }
        }

        function onAbort() {
            void close();
        }

        async function dispatchOne(reqRid) {
            let responded = false;
            function sendReply(status, headersList, bodyBytes) {
                if (responded) return;
                const res = opRespond(reqRid, status, headersList, bodyBytes);
                if (!res || res.ok !== true) {
                    throwFailure(res || { name: "Error", message: "opRespond failed" });
                }
                responded = true;
            }

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
                const resState = getResponseInternal(res);
                sendReply(res.status, resState.headersList, resState.bodyBytes);
            } catch (err) {
                if (!responded) {
                    try {
                        const fallback =
                            err instanceof HttpError
                                ? errorToResponse(err, req)
                                : internalServerErrorResponse(req);
                        const fbState = getResponseInternal(fallback);
                        sendReply(fallback.status, fbState.headersList, fbState.bodyBytes);
                    } catch {
                        // Fall through to finally safety reply.
                    }
                }
            } finally {
                if (!responded) {
                    try {
                        opRespond(
                            reqRid,
                            500,
                            [["content-type", "text/plain; charset=utf-8"]],
                            "Internal Server Error",
                        );
                    } catch {
                        // Last-resort guard.
                    }
                    responded = true;
                }
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
                    setAdd(inFlight, task);
                    promiseFinally(task, () => {
                        setDelete(inFlight, task);
                    });
                }
            } finally {
                if (setSize(inFlight) > 0) {
                    await promiseAllSettled(arrayFrom(setValues(inFlight)));
                }
                if (closeOpPromise) {
                    await closeOpPromise;
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
            closeOpPromise = opClose(serverRid);
            closingPromise = (async () => {
                await closeOpPromise;
                await finished;
            })();
            return closingPromise;
        }

        try {
            if (signal) {
                if (signal.aborted) {
                    void close();
                } else {
                    signal.addEventListener("abort", onAbort, { once: true });
                }
            }
        } catch (err) {
            void close();
            throw err;
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
                __proto__: null,
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
