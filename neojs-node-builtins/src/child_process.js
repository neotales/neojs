// node:child_process — opt-in, synchronous, bounded subset.
//
// This module is served only by a runtime constructed with Node compatibility.
// It calls the private host bridge installed by that runtime. The bridge runs
// the engine-neutral `neojs-node-builtins` core, which enforces an exact
// executable allowlist, an empty inherited environment, and finite time, input,
// output, argument, and environment limits.
//
// Supported: spawnSync, execFileSync, with encoding: "utf8" required.
// Not implemented: spawn, exec, execFile, execSync, fork, ChildProcess, and
// every callback, stream, and Promise form. Those names throw a clear error;
// none is faked with a blocking call or a resolved Promise.
"use strict";

// The bridge is installed before user code with the original JSON.stringify,
// record/array constructors, Array.isArray, and own-property helper. Requests
// are built from null-prototype records and arrays, so inherited toJSON hooks,
// getters, and a replaced global JSON.stringify are not consulted.
const bridge = globalThis.__neojsNodeChildProcess;
if (
    !bridge ||
    typeof bridge.runSync !== "function" ||
    typeof bridge.stringify !== "function" ||
    typeof bridge.record !== "function" ||
    typeof bridge.list !== "function" ||
    typeof bridge.isArray !== "function" ||
    typeof bridge.hasOwn !== "function"
) {
    throw new Error("node:child_process is not installed in this runtime");
}
const runSync = bridge.runSync;
const stringify = bridge.stringify;
const record = bridge.record;
const list = bridge.list;
const isArray = bridge.isArray;
const hasOwn = bridge.hasOwn;

const SUPPORTED_OPTIONS = record();
SUPPORTED_OPTIONS.cwd = true;
SUPPORTED_OPTIONS.env = true;
SUPPORTED_OPTIONS.input = true;
SUPPORTED_OPTIONS.timeout = true;
SUPPORTED_OPTIONS.maxBuffer = true;
SUPPORTED_OPTIONS.encoding = true;

// Hard ceilings before guest normalization and before the bridge. The Rust
// policy may be stricter; these bound copies and the serialized request.
const MAX_REQUEST_CHARS = 4_194_304;
const MAX_INPUT_CHARS = 1_048_576;
const MAX_ARGS = 256;
const MAX_ARG_CHARS = 65_536;
const MAX_ENV_ENTRIES = 128;
const MAX_ENV_CHARS = 65_536;
const MAX_PATH_CHARS = 4_096;

function requestTooLarge() {
    return makeError(
        "ERR_NODE_REQUEST_TOO_LARGE",
        "the node:child_process request exceeds the hard size limit",
    );
}

function makeError(code, message, properties) {
    const error = new Error(message);
    Object.defineProperty(error, "code", {
        value: code,
        enumerable: true,
        configurable: true,
        writable: true,
    });
    if (properties !== undefined) {
        for (const key of Object.keys(properties)) {
            const value = properties[key];
            if (value !== undefined) {
                Object.defineProperty(error, key, {
                    value,
                    enumerable: true,
                    configurable: true,
                    writable: true,
                });
            }
        }
    }
    return error;
}

function unsupported(name) {
    return makeError(
        "ERR_NODE_UNSUPPORTED",
        `node:child_process.${name} is not implemented; use spawnSync or execFileSync with encoding: "utf8"`,
    );
}

function unsupportedFunction(name) {
    return function unsupportedOperation() {
        throw unsupported(name);
    };
}

function invalid(message) {
    return makeError("ERR_INVALID_ARG_VALUE", message);
}

function requireString(value, what) {
    if (typeof value !== "string") {
        throw makeError("ERR_INVALID_ARG_TYPE", `${what} must be a string`);
    }
    return value;
}

function normalizeOptions(options) {
    if (options === undefined) {
        options = record();
    }
    if (options === null || typeof options !== "object" || isArray(options)) {
        throw makeError("ERR_INVALID_ARG_TYPE", "options must be an object");
    }
    for (const key in options) {
        if (!hasOwn(options, key)) continue;
        if (SUPPORTED_OPTIONS[key] !== true) {
            throw makeError(
                "ERR_NODE_UNSUPPORTED",
                `the node:child_process option \`${key}\` is not implemented`,
            );
        }
    }

    const encoding = options.encoding;
    const cwdOption = options.cwd;
    const envOption = options.env;
    const inputOption = options.input;
    const timeoutOption = options.timeout;
    const maxBufferOption = options.maxBuffer;

    if (typeof encoding !== "string" || encoding.toLowerCase().replace("-", "") !== "utf8") {
        throw makeError(
            "ERR_NODE_UNSUPPORTED_ENCODING",
            'node:child_process requires encoding: "utf8" in this opt-in slice',
        );
    }

    const normalized = record();
    if (cwdOption !== undefined) {
        const cwd = requireString(cwdOption, "options.cwd");
        if (cwd.length > MAX_PATH_CHARS) throw requestTooLarge();
        if (cwd.includes("\0")) {
            throw invalid("options.cwd must not contain a NUL byte");
        }
        normalized.cwd = cwd;
    }
    if (envOption !== undefined) {
        const env = envOption;
        if (env === null || typeof env !== "object" || isArray(env)) {
            throw makeError("ERR_INVALID_ARG_TYPE", "options.env must be an object");
        }
        let count = 0;
        let total = 0;
        const copy = record();
        for (const name in env) {
            if (!hasOwn(env, name)) continue;
            count += 1;
            if (count > MAX_ENV_ENTRIES) throw requestTooLarge();
            if (name.length === 0 || name.includes("=") || name.includes("\0")) {
                throw invalid("an environment name is not valid");
            }
            if (name.length > MAX_PATH_CHARS) throw requestTooLarge();
            const value = env[name];
            if (typeof value !== "string") {
                throw makeError("ERR_INVALID_ARG_TYPE", "environment values must be strings");
            }
            if (value.includes("\0")) {
                throw invalid("an environment value must not contain a NUL byte");
            }
            total += name.length + value.length;
            if (total > MAX_ENV_CHARS) throw requestTooLarge();
            copy[name] = value;
        }
        normalized.env = copy;
    }
    if (inputOption !== undefined) {
        if (typeof inputOption !== "string") {
            throw makeError(
                "ERR_NODE_UNSUPPORTED",
                "node:child_process input must be a string in this opt-in slice",
            );
        }
        if (inputOption.length > MAX_INPUT_CHARS) throw requestTooLarge();
        normalized.input = inputOption;
    }
    if (timeoutOption !== undefined) {
        if (!Number.isInteger(timeoutOption) || timeoutOption < 0) {
            throw invalid("options.timeout must be a non-negative integer");
        }
        normalized.timeout = timeoutOption;
    }
    if (maxBufferOption !== undefined) {
        if (!Number.isInteger(maxBufferOption) || maxBufferOption <= 0) {
            throw invalid("options.maxBuffer must be a positive integer");
        }
        normalized.maxBuffer = maxBufferOption;
    }
    return normalized;
}

function normalizeArgs(args) {
    if (args === undefined) {
        return list();
    }
    if (!isArray(args)) {
        throw makeError("ERR_INVALID_ARG_TYPE", "arguments must be an array");
    }
    const length = args.length;
    if (!Number.isInteger(length) || length < 0) {
        throw makeError("ERR_INVALID_ARG_TYPE", "arguments must have a valid length");
    }
    if (length > MAX_ARGS) throw requestTooLarge();
    let total = 0;
    const normalized = list();
    for (let index = 0; index < length; index += 1) {
        const argument = args[index];
        if (typeof argument === "string") {
            if (argument.includes("\0")) {
                throw invalid("an argument must not contain a NUL byte");
            }
            total += argument.length;
            if (total > MAX_ARG_CHARS) throw requestTooLarge();
            normalized[normalized.length] = argument;
        } else if (typeof argument === "number" || typeof argument === "boolean") {
            const text = String(argument);
            total += text.length;
            if (total > MAX_ARG_CHARS) throw requestTooLarge();
            normalized[normalized.length] = text;
        } else {
            throw makeError(
                "ERR_INVALID_ARG_TYPE",
                "arguments must be strings, numbers, or booleans",
            );
        }
    }
    return normalized;
}

function normalizeCall(file, args, options) {
    const program = requireString(file, "file");
    if (program.length === 0) {
        throw invalid("file must not be empty");
    }
    if (program.length > MAX_PATH_CHARS) throw requestTooLarge();
    if (program.includes("\0")) {
        throw invalid("file must not contain a NUL byte");
    }
    const normalizedArgs = normalizeArgs(args);
    const normalizedOptions = normalizeOptions(options);
    const result = record();
    result.program = program;
    result.args = normalizedArgs;
    result.options = normalizedOptions;
    return result;
}

function call(operation, call_) {
    const request = record();
    request.operation = operation;
    request.file = call_.program;
    request.args = call_.args;
    request.options = call_.options;
    let text;
    try {
        text = stringify(request);
    } catch {
        throw makeError(
            "ERR_INVALID_ARG_VALUE",
            "the node:child_process request could not be serialized",
        );
    }
    if (text.length > MAX_REQUEST_CHARS) throw requestTooLarge();
    try {
        text = runSync(text);
    } catch {
        throw makeError(
            "ERR_NODE_BRIDGE_FAILURE",
            "the node:child_process host bridge failed",
        );
    }
    let response;
    try {
        response = JSON.parse(text);
    } catch {
        throw makeError(
            "ERR_NODE_BRIDGE_FAILURE",
            "the node:child_process host bridge returned an invalid response",
        );
    }
    if (response === null || typeof response !== "object") {
        throw makeError(
            "ERR_NODE_BRIDGE_FAILURE",
            "the node:child_process host bridge returned an invalid response",
        );
    }
    if (response.ok !== true) {
        const code =
            response.error && typeof response.error.code === "string"
                ? response.error.code
                : "ERR_NODE_CHILD_PROCESS";
        const message =
            response.error && typeof response.error.message === "string"
                ? response.error.message
                : "the child process failed";
        throw makeError(code, message);
    }
    const result = response.result;
    if (result === null || typeof result !== "object") {
        throw makeError(
            "ERR_NODE_BRIDGE_FAILURE",
            "the node:child_process host bridge returned an invalid result",
        );
    }
    if (
        typeof result.pid !== "number" ||
        typeof result.stdout !== "string" ||
        typeof result.stderr !== "string" ||
        (result.status !== null && typeof result.status !== "number") ||
        (result.signalName !== null && typeof result.signalName !== "string") ||
        typeof result.timedOut !== "boolean" ||
        typeof result.limitExceeded !== "boolean" ||
        typeof result.incomplete !== "boolean" ||
        typeof result.ioError !== "boolean"
    ) {
        throw makeError(
            "ERR_NODE_BRIDGE_FAILURE",
            "the node:child_process host bridge returned an invalid result",
        );
    }
    return result;
}

function resultFields(result) {
    return {
        pid: result.pid,
        output: [null, result.stdout, result.stderr],
        stdout: result.stdout,
        stderr: result.stderr,
        status: result.status,
        signal: result.signalName,
    };
}

/**
 * Runs a program to completion and returns a Node-shaped result object.
 *
 * The result has `pid`, `output`, `stdout`, `stderr`, `status`, and `signal`.
 * A timeout or output overflow sets `error` and leaves `status` null. A spawn
 * failure that Node reports as `error` (ENOENT or EACCES) is returned the same
 * way; a policy denial or argument error throws.
 */
function spawnSync(file, args, options) {
    if (args !== undefined && !isArray(args)) {
        options = args;
        args = [];
    }
    const call_ = normalizeCall(file, args, options);
    let result;
    try {
        result = call("spawnSync", call_);
    } catch (error) {
        if (error && (error.code === "ENOENT" || error.code === "EACCES")) {
            return {
                pid: 0,
                output: [null, "", ""],
                stdout: "",
                stderr: "",
                status: null,
                signal: null,
                error,
            };
        }
        throw error;
    }

    const fields = resultFields(result);
    if (result.timedOut) {
        fields.error = makeError("ETIMEDOUT", "child process timed out");
    } else if (result.limitExceeded) {
        fields.error = makeError(
            "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
            "child process output exceeded maxBuffer",
        );
    } else if (result.incomplete) {
        fields.error = makeError(
            "ERR_NODE_CHILD_PROCESS_INCOMPLETE",
            "the direct child exited while a descendant still held a piped stream; output may be incomplete",
        );
    } else if (result.ioError) {
        fields.error = makeError(
            "ERR_NODE_CHILD_PROCESS_IO",
            "a child process pipe read failed; output may be incomplete",
        );
    }
    return fields;
}

/**
 * Runs a program to completion and returns its standard output as a string.
 *
 * A non-zero exit, a signal, a timeout, an output overflow, a spawn failure, or
 * a policy denial throws. The thrown error carries the same result fields as
 * `spawnSync` where they exist.
 */
function execFileSync(file, args, options) {
    if (args !== undefined && !isArray(args)) {
        options = args;
        args = [];
    }
    const call_ = normalizeCall(file, args, options);
    const result = call("execFileSync", call_);
    const fields = resultFields(result);
    if (result.timedOut) {
        throw makeError("ETIMEDOUT", "child process timed out", fields);
    }
    if (result.limitExceeded) {
        throw makeError(
            "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
            "child process output exceeded maxBuffer",
            fields,
        );
    }
    if (result.incomplete) {
        throw makeError(
            "ERR_NODE_CHILD_PROCESS_INCOMPLETE",
            "the direct child exited while a descendant still held a piped stream; output may be incomplete",
            fields,
        );
    }
    if (result.ioError) {
        throw makeError(
            "ERR_NODE_CHILD_PROCESS_IO",
            "a child process pipe read failed; output may be incomplete",
            fields,
        );
    }
    if (result.status !== 0) {
        throw makeError(
            "ERR_NODE_CHILD_PROCESS_EXIT",
            "child process exited with a non-zero status",
            fields,
        );
    }
    return result.stdout;
}

class ChildProcess {
    constructor() {
        throw unsupported("ChildProcess");
    }
}

const spawn = unsupportedFunction("spawn");
const exec = unsupportedFunction("exec");
const execFile = unsupportedFunction("execFile");
const execSync = unsupportedFunction("execSync");
const fork = unsupportedFunction("fork");

const api = Object.freeze({
    spawn,
    spawnSync,
    exec,
    execFile,
    execFileSync,
    execSync,
    fork,
    ChildProcess,
});

export { ChildProcess, exec, execFile, execFileSync, execSync, fork, spawn, spawnSync };
export default api;
