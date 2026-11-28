(function createVaelExec(host) {
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
    const arrayIsArray = Array.isArray;
    const numberIsInteger = Number.isInteger;
    const numberIsFinite = Number.isFinite;
    const jsonParse = JSON.parse;
    const callBind = Function.prototype.call.bind(Function.prototype.bind);
    const fnCallBind = (fn) => callBind(Function.prototype.call, fn);
    const hasOwn = fnCallBind(Object.prototype.hasOwnProperty);
    const stringIncludes = fnCallBind(String.prototype.includes);
    const symbolDispose = typeof Symbol.dispose === "symbol"
        ? Symbol.dispose
        : Symbol.for("Symbol.dispose");

    const typedArrayProto = getPrototypeOf(makeUint8Array.prototype);
    const typedArraySet = fnCallBind(typedArrayProto.set);
    const typedArrayByteLength = fnCallBind(
        getOwnPropertyDescriptor(typedArrayProto, "byteLength").get,
    );
    const typedArrayToStringTag = fnCallBind(
        getOwnPropertyDescriptor(typedArrayProto, Symbol.toStringTag).get,
    );
    const typedArrayIndexOf = fnCallBind(typedArrayProto.indexOf);

    const MAX_OUTPUT_BYTES = host.maxOutputBytes;
    const hostValidateSignal = host.validateSignal;
    const hostValidateTimeoutMs = host.validateTimeoutMs;
    const hostParseStringArray = host.parseStringArray;
    const hostParseEnvObject = host.parseEnvObject;
    const hostParseCommandOptions = host.parseCommandOptions;
    const hostExecSync = host.execSync;
    const hostRunSync = host.runSync;
    const hostWhichSync = host.whichSync;
    const hostWhichAllSync = host.whichAllSync;
    const hostLockFileSync = host.lockFileSync;
    const hostLockUnlock = host.lockUnlock;
    const hostLockIsLocked = host.lockIsLocked;
    const hostDecodeUtf8Lossy = host.decodeUtf8Lossy;
    const hostLinesLossy = host.linesLossy;

    const INTERNAL_TOKEN = objectCreate(null);

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

    function pushOwn(arr, value) {
        defineOwn(arr, arr.length, value, true, true);
    }

    class CommandError extends makeError {
        constructor(message, code = "VAEL_EXEC_IO", options = undefined) {
            super(message === undefined ? "" : makeString(message));
            defineOwn(this, "name", "CommandError", false, true);
            defineOwn(
                this,
                "code",
                code === undefined ? "VAEL_EXEC_IO" : makeString(code),
                true,
                false,
            );
            if (options && typeof options === "object") {
                if (
                    hasOwn(options, "exitCode") &&
                    options.exitCode !== undefined &&
                    options.exitCode !== null
                ) {
                    defineOwn(this, "exitCode", options.exitCode, true, false);
                }
                if (
                    hasOwn(options, "signal") &&
                    options.signal !== undefined &&
                    options.signal !== null
                ) {
                    defineOwn(this, "signal", options.signal, true, false);
                }
                if (
                    hasOwn(options, "fileName") &&
                    options.fileName !== undefined &&
                    options.fileName !== null
                ) {
                    defineOwn(this, "fileName", options.fileName, true, false);
                }
            }
        }
    }

    class NotFoundOnPathError extends makeError {
        constructor(fileName, message = undefined) {
            const file = fileName === undefined ? "" : makeString(fileName);
            const msg = message !== undefined
                ? makeString(message)
                : `executable not found on PATH: ${file}`;
            super(msg);
            defineOwn(this, "name", "NotFoundOnPathError", false, true);
            defineOwn(this, "code", "VAEL_EXEC_NOT_FOUND", true, false);
            defineOwn(this, "fileName", file, true, false);
        }
    }

    function throwExecFailure(res, spawnFileName) {
        if (res.notFound && spawnFileName !== undefined) {
            throw new NotFoundOnPathError(spawnFileName, res.message);
        }
        const opts = objectCreate(null);
        if (res.fileName !== undefined) {
            defineOwn(opts, "fileName", res.fileName, true, true);
        } else if (spawnFileName !== undefined && res.code === "VAEL_EXEC_PERMISSION_DENIED") {
            defineOwn(opts, "fileName", spawnFileName, true, true);
        }
        throw new CommandError(res.message, res.code, opts);
    }

    function assertValidProgram(program) {
        if (typeof program !== "string") {
            throw new makeTypeError("program must be a string");
        }
        if (program.length === 0) {
            throw new CommandError(
                "program name must not be empty",
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        if (stringIncludes(program, "\0")) {
            throw new CommandError(
                "program name must not contain a NUL byte",
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        return program;
    }

    function assertValidArg(arg) {
        if (typeof arg !== "string") {
            throw new makeTypeError("argument must be a string");
        }
        if (stringIncludes(arg, "\0")) {
            throw new CommandError(
                "process argument must not contain a NUL byte",
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        return arg;
    }

    function assertValidEnvKey(key) {
        if (typeof key !== "string") {
            throw new makeTypeError("environment variable key must be a string");
        }
        if (key.length === 0 || stringIncludes(key, "=") || stringIncludes(key, "\0")) {
            throw new CommandError(
                "environment variable key must be non-empty and contain neither '=' nor NUL",
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        return key;
    }

    function validateSignalOrThrow(sig) {
        if (typeof sig !== "string") {
            throw new makeTypeError("killSignal must be a string");
        }
        const res = hostValidateSignal(sig);
        if (!res.ok) {
            throwExecFailure(res);
        }
        return res.signal;
    }

    function validateTimeoutOrThrow(ms) {
        if (typeof ms !== "number") {
            throw new makeTypeError("timeout must be a number");
        }
        if (!numberIsFinite(ms) || ms <= 0) {
            throw new CommandError(
                "timeout must be greater than zero",
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        const res = hostValidateTimeoutMs(ms);
        if (!res.ok) {
            throwExecFailure(res);
        }
        return ms;
    }

    function validateOutputMode(mode, label) {
        if (typeof mode !== "string") {
            throw new makeTypeError(`${label} mode must be a string`);
        }
        if (mode !== "inherit" && mode !== "piped" && mode !== "null") {
            throw new CommandError(
                `${label} mode must be "inherit", "piped", or "null"`,
                "VAEL_EXEC_INVALID_ARGUMENT",
            );
        }
        return mode;
    }

    class Output {
        #stdout;
        #stderr;
        #code;
        #signal;
        #success;
        #timedOut;
        #truncated;

        constructor(raw) {
            if (!raw || typeof raw !== "object" || !hasOwn(raw, "stdoutBytes")) {
                throw new makeTypeError("Output requires valid output data");
            }
            const stdoutCopy = copyBytes(raw.stdoutBytes);
            const stderrCopy = copyBytes(raw.stderrBytes);
            this.#stdout = stdoutCopy;
            this.#stderr = stderrCopy;
            this.#code = raw.code;
            this.#signal = raw.signal;
            this.#success = Boolean(raw.success);
            this.#timedOut = Boolean(raw.timedOut);
            this.#truncated = Boolean(raw.truncated);

            const stdoutDesc = objectCreate(null);
            stdoutDesc.get = () => copyBytes(this.#stdout);
            stdoutDesc.enumerable = true;
            stdoutDesc.configurable = true;
            defineProperty(this, "stdout", stdoutDesc);

            const stderrDesc = objectCreate(null);
            stderrDesc.get = () => copyBytes(this.#stderr);
            stderrDesc.enumerable = true;
            stderrDesc.configurable = true;
            defineProperty(this, "stderr", stderrDesc);

            defineOwn(this, "code", this.#code, true, false);
            defineOwn(this, "signal", this.#signal, true, false);
            defineOwn(this, "success", this.#success, true, false);
            defineOwn(this, "timedOut", this.#timedOut, true, false);
            defineOwn(this, "truncated", this.#truncated, true, false);
            freeze(this);
        }

        text() {
            return hostDecodeUtf8Lossy(this.#stdout);
        }

        lines() {
            return hostLinesLossy(this.#stdout);
        }

        json() {
            return jsonParse(hostDecodeUtf8Lossy(this.#stdout));
        }

        errorText() {
            return hostDecodeUtf8Lossy(this.#stderr);
        }

        errorLines() {
            return hostLinesLossy(this.#stderr);
        }

        errorJson() {
            return jsonParse(hostDecodeUtf8Lossy(this.#stderr));
        }

        validate(predicate = undefined, failOnStderr = false) {
            if (failOnStderr !== undefined && typeof failOnStderr !== "boolean") {
                throw new makeTypeError("failOnStderr must be a boolean");
            }
            let passed;
            if (predicate === undefined || predicate === null) {
                passed = this.#success;
            } else if (typeof predicate === "function") {
                passed = Boolean(predicate(this));
            } else if (typeof predicate === "number") {
                passed = !this.#timedOut && this.#code === predicate;
            } else if (arrayIsArray(predicate)) {
                passed = false;
                if (!this.#timedOut) {
                    for (let i = 0; i < predicate.length; i++) {
                        if (hasOwn(predicate, i) && predicate[i] === this.#code) {
                            passed = true;
                            break;
                        }
                    }
                }
            } else {
                throw new makeTypeError(
                    "predicate must be a function, exit code, or array of exit codes",
                );
            }

            if (!passed) {
                const code = this.#timedOut ? "VAEL_EXEC_TIMEOUT" : "VAEL_EXEC_NONZERO_EXIT";
                const msg = this.#timedOut
                    ? "command timed out"
                    : this.#signal !== null
                    ? `command terminated by signal ${this.#signal}`
                    : `command failed with exit code ${this.#code}`;
                throw new CommandError(msg, code, {
                    exitCode: this.#code,
                    signal: this.#signal === null ? undefined : this.#signal,
                });
            }

            if (failOnStderr === true && typedArrayByteLength(this.#stderr) > 0) {
                throw new CommandError(
                    "command produced unexpected output on stderr",
                    "VAEL_EXEC_STDERR",
                    {
                        exitCode: this.#code,
                        signal: this.#signal === null ? undefined : this.#signal,
                    },
                );
            }

            return this;
        }
    }

    class Command {
        #spec;

        constructor(programOrArgv, argsOrOptions = undefined, maybeOptions = undefined) {
            let program;
            let initialArgs = [];
            let rawOptions;

            if (arrayIsArray(programOrArgv)) {
                if (maybeOptions !== undefined) {
                    throw new makeTypeError(
                        "Command(argv, options?) takes at most two arguments",
                    );
                }
                const parsed = hostParseStringArray(
                    programOrArgv,
                    "argv",
                );
                if (!parsed.ok) {
                    throwExecFailure(parsed);
                }
                if (parsed.items.length === 0) {
                    throw new CommandError(
                        "program name must not be empty",
                        "VAEL_EXEC_INVALID_ARGUMENT",
                    );
                }
                program = assertValidProgram(parsed.items[0]);
                for (let i = 1; i < parsed.items.length; i++) {
                    pushOwn(initialArgs, parsed.items[i]);
                }
                rawOptions = argsOrOptions;
            } else if (typeof programOrArgv === "string") {
                program = assertValidProgram(programOrArgv);
                if (argsOrOptions === undefined) {
                    rawOptions = maybeOptions;
                } else if (arrayIsArray(argsOrOptions)) {
                    const parsed = hostParseStringArray(argsOrOptions, "args");
                    if (!parsed.ok) {
                        throwExecFailure(parsed);
                    }
                    initialArgs = parsed.items;
                    rawOptions = maybeOptions;
                } else if (
                    maybeOptions === undefined &&
                    typeof argsOrOptions === "object" &&
                    argsOrOptions !== null
                ) {
                    rawOptions = argsOrOptions;
                } else {
                    throw new makeTypeError("args must be an array of strings");
                }
            } else {
                throw new makeTypeError(
                    "Command requires a non-empty program string or argv array",
                );
            }

            const spec = objectCreate(null);
            defineOwn(spec, "program", program, true, true);
            defineOwn(spec, "args", initialArgs, true, true);
            defineOwn(spec, "cwd", undefined, true, true);
            defineOwn(spec, "clearEnv", false, true, true);
            defineOwn(spec, "envOps", [], true, true);
            defineOwn(spec, "stdin", undefined, true, true);
            defineOwn(spec, "stdout", undefined, true, true);
            defineOwn(spec, "stderr", undefined, true, true);
            defineOwn(spec, "timeoutMs", undefined, true, true);
            defineOwn(spec, "maxOutputBytes", undefined, true, true);
            defineOwn(spec, "killSignal", undefined, true, true);
            defineOwn(spec, "killGraceMs", undefined, true, true);
            this.#spec = spec;

            if (rawOptions !== undefined) {
                const parsedOpts = hostParseCommandOptions(rawOptions);
                if (!parsedOpts.ok) {
                    throwExecFailure(parsedOpts);
                }
                if (parsedOpts.args !== undefined) {
                    for (let i = 0; i < parsedOpts.args.length; i++) {
                        pushOwn(spec.args, parsedOpts.args[i]);
                    }
                }
                if (parsedOpts.cwd !== undefined) {
                    spec.cwd = parsedOpts.cwd;
                }
                if (parsedOpts.clearEnv === true) {
                    this.envClear();
                }
                if (parsedOpts.env !== undefined) {
                    for (let i = 0; i < parsedOpts.env.length; i++) {
                        const pair = parsedOpts.env[i];
                        pushOwn(spec.envOps, ["set", pair[0], pair[1]]);
                    }
                }
                if (parsedOpts.envRemove !== undefined) {
                    for (let i = 0; i < parsedOpts.envRemove.length; i++) {
                        pushOwn(spec.envOps, ["remove", parsedOpts.envRemove[i]]);
                    }
                }
                if (parsedOpts.stdin !== undefined) {
                    spec.stdin = parsedOpts.stdin;
                }
                if (parsedOpts.stdout !== undefined) {
                    spec.stdout = parsedOpts.stdout;
                }
                if (parsedOpts.stderr !== undefined) {
                    spec.stderr = parsedOpts.stderr;
                }
                if (parsedOpts.timeoutMs !== undefined) {
                    spec.timeoutMs = parsedOpts.timeoutMs;
                }
                if (parsedOpts.maxOutputBytes !== undefined) {
                    spec.maxOutputBytes = parsedOpts.maxOutputBytes;
                }
                if (parsedOpts.killSignal !== undefined) {
                    spec.killSignal = parsedOpts.killSignal;
                }
                if (parsedOpts.killGraceMs !== undefined) {
                    spec.killGraceMs = parsedOpts.killGraceMs;
                }
            }
        }

        arg(v) {
            const spec = this.#spec;
            const checked = assertValidArg(v);
            pushOwn(spec.args, checked);
            return this;
        }

        args(vs) {
            const spec = this.#spec;
            if (!arrayIsArray(vs)) {
                throw new makeTypeError("args must be an array of strings");
            }
            const parsed = hostParseStringArray(vs, "args");
            if (!parsed.ok) {
                throwExecFailure(parsed);
            }
            for (let i = 0; i < parsed.items.length; i++) {
                pushOwn(spec.args, parsed.items[i]);
            }
            return this;
        }

        cwd(dir) {
            const spec = this.#spec;
            if (typeof dir !== "string") {
                throw new makeTypeError("cwd must be a string");
            }
            if (dir.length === 0 || stringIncludes(dir, "\0")) {
                throw new CommandError(
                    "working directory must not be empty or contain NUL",
                    "VAEL_EXEC_INVALID_ARGUMENT",
                );
            }
            spec.cwd = dir;
            return this;
        }

        env(k, v) {
            const spec = this.#spec;
            assertValidEnvKey(k);
            if (typeof v !== "string") {
                throw new makeTypeError("environment variable value must be a string");
            }
            if (stringIncludes(v, "\0")) {
                throw new CommandError(
                    "environment variable value must not contain a NUL byte",
                    "VAEL_EXEC_INVALID_ARGUMENT",
                );
            }
            pushOwn(spec.envOps, ["set", k, v]);
            return this;
        }

        envs(obj) {
            const spec = this.#spec;
            const parsed = hostParseEnvObject(obj);
            if (!parsed.ok) {
                throwExecFailure(parsed);
            }
            for (let i = 0; i < parsed.pairs.length; i++) {
                const pair = parsed.pairs[i];
                pushOwn(spec.envOps, ["set", pair[0], pair[1]]);
            }
            return this;
        }

        envRemove(k) {
            const spec = this.#spec;
            assertValidEnvKey(k);
            pushOwn(spec.envOps, ["remove", k]);
            return this;
        }

        envClear() {
            const spec = this.#spec;
            spec.clearEnv = true;
            spec.envOps = [];
            return this;
        }

        stdin(modeOrData) {
            const spec = this.#spec;
            const entry = objectCreate(null);
            if (
                modeOrData === "inherit" ||
                modeOrData === "piped" ||
                modeOrData === "null"
            ) {
                defineOwn(entry, "kind", modeOrData, true, true);
            } else if (typeof modeOrData === "string") {
                defineOwn(entry, "kind", "text", true, true);
                defineOwn(entry, "data", modeOrData, true, true);
            } else {
                const copied = copyBytes(modeOrData);
                defineOwn(entry, "kind", "bytes", true, true);
                defineOwn(entry, "data", copied, true, true);
            }
            spec.stdin = entry;
            return this;
        }

        stdout(mode) {
            const spec = this.#spec;
            spec.stdout = validateOutputMode(mode, "stdout");
            return this;
        }

        stderr(mode) {
            const spec = this.#spec;
            spec.stderr = validateOutputMode(mode, "stderr");
            return this;
        }

        timeout(ms) {
            const spec = this.#spec;
            spec.timeoutMs = validateTimeoutOrThrow(ms);
            return this;
        }

        maxOutputBytes(n) {
            const spec = this.#spec;
            if (typeof n !== "number") {
                throw new makeTypeError("maxOutputBytes must be a number");
            }
            if (!numberIsInteger(n) || n < 0 || n > MAX_OUTPUT_BYTES) {
                throw new CommandError(
                    "maxOutputBytes exceeds hard limit of 64 MiB",
                    "VAEL_EXEC_INVALID_ARGUMENT",
                );
            }
            spec.maxOutputBytes = n;
            return this;
        }

        killSignal(sig) {
            const spec = this.#spec;
            spec.killSignal = validateSignalOrThrow(sig);
            return this;
        }

        killGrace(ms) {
            const spec = this.#spec;
            if (typeof ms !== "number") {
                throw new makeTypeError("killGrace must be a number");
            }
            if (!numberIsFinite(ms) || ms < 0) {
                throw new CommandError(
                    "killGrace must be a non-negative finite number",
                    "VAEL_EXEC_INVALID_ARGUMENT",
                );
            }
            spec.killGraceMs = ms;
            return this;
        }

        toArgs() {
            const spec = this.#spec;
            const out = [spec.program];
            for (let i = 0; i < spec.args.length; i++) {
                pushOwn(out, spec.args[i]);
            }
            return out;
        }

        outputSync() {
            const spec = this.#spec;
            const res = hostExecSync(spec);
            if (!res.ok) {
                throwExecFailure(res, spec.program);
            }
            return new Output(res);
        }

        execSync() {
            return this.outputSync();
        }

        statusSync() {
            const spec = this.#spec;
            const res = hostRunSync(spec);
            if (!res.ok) {
                throwExecFailure(res, spec.program);
            }
            const status = objectCreate(null);
            defineOwn(status, "code", res.code, true, false);
            defineOwn(status, "signal", res.signal, true, false);
            defineOwn(status, "success", res.success, true, false);
            defineOwn(status, "timedOut", res.timedOut, true, false);
            return freeze(status);
        }

        runSync() {
            return this.statusSync();
        }
    }

    class FileLock {
        #handle;
        #path;
        #mode;

        constructor(token, handle, path, mode) {
            if (token !== INTERNAL_TOKEN) {
                throw new makeTypeError(
                    "FileLock cannot be constructed directly; use lockFileSync()",
                );
            }
            this.#handle = handle;
            this.#path = path;
            this.#mode = mode;
        }

        get path() {
            return this.#path;
        }

        get mode() {
            return this.#mode;
        }

        get locked() {
            return hostLockIsLocked(this.#handle);
        }

        unlock() {
            const res = hostLockUnlock(this.#handle);
            if (!res.ok) {
                throwExecFailure(res);
            }
        }
    }

    defineOwn(FileLock.prototype, symbolDispose, FileLock.prototype.unlock, false, true);

    function cmd(programOrArgv, argsOrOptions, maybeOptions) {
        return new Command(programOrArgv, argsOrOptions, maybeOptions);
    }

    function execSync(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return programOrArgv.outputSync();
        }
        return new Command(programOrArgv, argsOrOptions, maybeOptions).outputSync();
    }

    function runSync(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return programOrArgv.statusSync();
        }
        return new Command(programOrArgv, argsOrOptions, maybeOptions).statusSync();
    }

    function whichSync(name, options) {
        if (typeof name !== "string") {
            throw new makeTypeError("executable name must be a string");
        }
        const res = hostWhichSync(name, options);
        if (!res.ok) {
            throwExecFailure(res);
        }
        return res.path;
    }

    function whichAllSync(name, options) {
        if (typeof name !== "string") {
            throw new makeTypeError("executable name must be a string");
        }
        const res = hostWhichAllSync(name, options);
        if (!res.ok) {
            throwExecFailure(res);
        }
        return res.paths;
    }

    function lockFileSync(path, options) {
        if (typeof path !== "string") {
            throw new makeTypeError("lock path must be a string");
        }
        const res = hostLockFileSync(path, options);
        if (!res.ok) {
            throwExecFailure(res);
        }
        return new FileLock(INTERNAL_TOKEN, res.handle, path, res.mode);
    }

    const mod = {};
    defineOwn(mod, "Command", Command, true, false);
    defineOwn(mod, "CommandError", CommandError, true, false);
    defineOwn(mod, "FileLock", FileLock, true, false);
    defineOwn(mod, "NotFoundOnPathError", NotFoundOnPathError, true, false);
    defineOwn(mod, "Output", Output, true, false);
    defineOwn(mod, "cmd", cmd, true, false);
    defineOwn(mod, "execSync", execSync, true, false);
    defineOwn(mod, "lockFileSync", lockFileSync, true, false);
    defineOwn(mod, "runSync", runSync, true, false);
    defineOwn(mod, "whichAllSync", whichAllSync, true, false);
    defineOwn(mod, "whichSync", whichSync, true, false);
    return freeze(mod);
})
