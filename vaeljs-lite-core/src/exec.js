(function createVaelExec(host) {
    "use strict";

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeString = String;
    const makeRegExp = RegExp;
    const makeUint8Array = Uint8Array;
    const makeMap = Map;
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
    const arraySlice = fnCallBind(Array.prototype.slice);
    const stringIncludes = fnCallBind(String.prototype.includes);
    const stringReplace = fnCallBind(String.prototype.replace);
    const stringToLowerCase = fnCallBind(String.prototype.toLowerCase);
    const stringToUpperCase = fnCallBind(String.prototype.toUpperCase);
    const mapGet = fnCallBind(makeMap.prototype.get);
    const mapSet = fnCallBind(makeMap.prototype.set);
    const mapHas = fnCallBind(makeMap.prototype.has);
    const mapDelete = fnCallBind(makeMap.prototype.delete);
    const mapClear = fnCallBind(makeMap.prototype.clear);
    const mapForEach = fnCallBind(makeMap.prototype.forEach);
    const symbolDispose = typeof Symbol.dispose === "symbol"
        ? Symbol.dispose
        : Symbol.for("Symbol.dispose");

    const CAMEL_BOUNDARY_RE = makeRegExp("([a-z0-9])([A-Z])", "g");
    const NON_ALNUM_RE = makeRegExp("[^a-zA-Z0-9]+", "g");
    const EDGE_UNDERSCORE_RE = makeRegExp("^_+|_+$", "g");

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
    const hostParsePathFinderOptions = host.parsePathFinderOptions;
    const hostIsExecutableFile = host.isExecutableFile;
    const hostEnvVar = host.envVar;
    const hostExpandEnvPath = host.expandEnvPath;
    const hostPlatform = host.platform;

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
        constructor(code = "VAEL_EXEC_IO", message = "", options = undefined) {
            super(message === undefined ? "" : makeString(message));
            defineOwn(this, "name", "CommandError", false, true);
            defineOwn(
                this,
                "code",
                code === undefined ? "VAEL_EXEC_IO" : makeString(code),
                true,
                false,
            );
            const hasOpts = options !== undefined && options !== null && typeof options === "object";
            const timedOut = hasOpts && hasOwn(options, "timedOut") && options.timedOut === true;
            defineOwn(this, "timedOut", timedOut, true, false);
            if (hasOpts) {
                if (
                    hasOwn(options, "exitCode") &&
                    options.exitCode !== undefined &&
                    options.exitCode !== null
                ) {
                    defineOwn(this, "exitCode", options.exitCode, true, false);
                }
                if (
                    hasOwn(options, "signal") &&
                    options.signal !== undefined
                ) {
                    defineOwn(this, "signal", options.signal, true, false);
                }
                if (hasOwn(options, "output") && options.output !== undefined) {
                    defineOwn(this, "output", options.output, true, false);
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
    freeze(CommandError.prototype);
    freeze(CommandError);

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
    freeze(NotFoundOnPathError.prototype);
    freeze(NotFoundOnPathError);

    function throwExecFailure(res, spawnFileName) {
        const message = hasOwn(res, "message") ? res.message : "";
        const code = hasOwn(res, "code") ? res.code : "VAEL_EXEC_IO";
        if (hasOwn(res, "notFound") && res.notFound && spawnFileName !== undefined) {
            throw new NotFoundOnPathError(spawnFileName, message);
        }
        const opts = objectCreate(null);
        if (hasOwn(res, "fileName") && res.fileName !== undefined) {
            defineOwn(opts, "fileName", res.fileName, true, true);
        } else if (spawnFileName !== undefined && code === "VAEL_EXEC_PERMISSION_DENIED") {
            defineOwn(opts, "fileName", spawnFileName, true, true);
        }
        throw new CommandError(code, message, opts);
    }

    function assertValidProgram(program) {
        if (typeof program !== "string") {
            throw new makeTypeError("program must be a string");
        }
        if (program.length === 0) {
            throw new CommandError(
                "VAEL_EXEC_INVALID_ARGUMENT",
                "program name must not be empty",
            );
        }
        if (stringIncludes(program, "\0")) {
            throw new CommandError(
                "VAEL_EXEC_INVALID_ARGUMENT",
                "program name must not contain a NUL byte",
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
                "VAEL_EXEC_INVALID_ARGUMENT",
                "process argument must not contain a NUL byte",
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
                "VAEL_EXEC_INVALID_ARGUMENT",
                "environment variable key must be non-empty and contain neither '=' nor NUL",
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
                "VAEL_EXEC_INVALID_ARGUMENT",
                "timeout must be greater than zero",
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
                "VAEL_EXEC_INVALID_ARGUMENT",
                `${label} mode must be "inherit", "piped", or "null"`,
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

        constructor(token, raw) {
            if (token !== INTERNAL_TOKEN) {
                throw new makeTypeError(
                    "Output cannot be constructed directly; use execSync() or Command.outputSync()",
                );
            }
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
            if (arguments.length > 2) {
                throw new makeTypeError("validate takes at most two arguments");
            }
            if (
                predicate !== undefined &&
                predicate !== null &&
                typeof predicate !== "function"
            ) {
                throw new makeTypeError("validate predicate must be a function");
            }
            if (failOnStderr !== undefined && typeof failOnStderr !== "boolean") {
                throw new makeTypeError("validate failOnStderr must be a boolean");
            }

            const passed =
                predicate !== undefined && predicate !== null
                    ? Boolean(predicate(this.#code))
                    : this.#success;

            if (!passed) {
                const code = this.#timedOut ? "VAEL_EXEC_TIMEOUT" : "VAEL_EXEC_NON_ZERO";
                const msg = this.#timedOut
                    ? "command timed out"
                    : this.#signal !== null
                    ? `command terminated by signal ${this.#signal}`
                    : `command failed with exit code ${this.#code}`;
                throw new CommandError(code, msg, {
                    __proto__: null,
                    exitCode: this.#code,
                    signal: this.#signal,
                    timedOut: this.#timedOut,
                    output: this,
                });
            }

            if (failOnStderr === true && typedArrayByteLength(this.#stderr) > 0) {
                throw new CommandError(
                    "VAEL_EXEC_STDERR",
                    "command produced unexpected output on stderr",
                    {
                        __proto__: null,
                        exitCode: this.#code,
                        signal: this.#signal,
                        timedOut: this.#timedOut,
                        output: this,
                    },
                );
            }

            return this;
        }
    }
    freeze(Output.prototype);
    freeze(Output);

    const globalLoggers = new makeMap();

    function createLogSubscription(registry, handler) {
        if (typeof handler !== "function") {
            throw new makeTypeError("log handler must be a function");
        }
        const token = objectCreate(null);
        const entry = objectCreate(null);
        entry.handler = handler;
        entry.active = true;
        mapSet(registry, token, entry);
        const dispose = () => {
            if (entry.active) {
                entry.active = false;
                mapDelete(registry, token);
            }
        };
        const sub = objectCreate(null);
        const activeDesc = objectCreate(null);
        activeDesc.get = () => entry.active;
        activeDesc.enumerable = true;
        activeDesc.configurable = true;
        defineProperty(sub, "active", activeDesc);
        defineOwn(sub, "dispose", dispose, true, false);
        defineOwn(sub, symbolDispose, dispose, false, false);
        return freeze(sub);
    }

    function clearLogRegistry(registry) {
        mapForEach(registry, (entry) => {
            entry.active = false;
        });
        mapClear(registry);
    }

    function appendActiveHandlers(registry, out) {
        mapForEach(registry, (entry) => {
            if (entry.active) {
                pushOwn(out, entry.handler);
            }
        });
    }

    function addLogger(handler) {
        if (arguments.length !== 1) {
            throw new makeTypeError("addLogger takes exactly one argument");
        }
        return createLogSubscription(globalLoggers, handler);
    }

    function clearLoggers() {
        if (arguments.length !== 0) {
            throw new makeTypeError("clearLoggers takes no arguments");
        }
        clearLogRegistry(globalLoggers);
    }

    function toUnderscoreUpper(name) {
        const camelSplit = stringReplace(name, CAMEL_BOUNDARY_RE, "$1_$2");
        const underscored = stringReplace(camelSplit, NON_ALNUM_RE, "_");
        const trimmed = stringReplace(underscored, EDGE_UNDERSCORE_RE, "");
        return stringToUpperCase(trimmed);
    }

    function clonePathFinderOptions(entry) {
        const copy = { name: entry.name };
        if (entry.executable !== undefined) copy.executable = entry.executable;
        if (entry.envVariable !== undefined) copy.envVariable = entry.envVariable;
        if (entry.cached !== undefined) copy.cached = entry.cached;
        if (entry.noCache !== undefined) copy.noCache = entry.noCache;
        if (entry.windows !== undefined) copy.windows = arraySlice(entry.windows);
        if (entry.linux !== undefined) copy.linux = arraySlice(entry.linux);
        if (entry.darwin !== undefined) copy.darwin = arraySlice(entry.darwin);
        return copy;
    }

    function hasPathSeparator(p) {
        return stringIncludes(p, "/") || stringIncludes(p, "\\");
    }

    function validatePathFinderKey(name) {
        if (typeof name !== "string" || name.length === 0 || stringIncludes(name, "\0")) {
            throw new makeTypeError("executable name must be a non-empty string");
        }
    }

    class PathFinder {
        #map = new makeMap();

        set(name, options) {
            if (arguments.length !== 2) {
                throw new makeTypeError("PathFinder.set takes exactly two arguments");
            }
            validatePathFinderKey(name);
            const parsed = hostParsePathFinderOptions(options);
            if (!parsed.ok) {
                throwExecFailure(parsed);
            }
            const stored = objectCreate(null);
            stored.name = parsed.name;
            if (parsed.executable !== undefined) stored.executable = parsed.executable;
            if (parsed.envVariable !== undefined) stored.envVariable = parsed.envVariable;
            if (parsed.cached !== undefined) stored.cached = parsed.cached;
            if (parsed.noCache !== undefined) stored.noCache = parsed.noCache;
            if (parsed.windows !== undefined) stored.windows = arraySlice(parsed.windows);
            if (parsed.linux !== undefined) stored.linux = arraySlice(parsed.linux);
            if (parsed.darwin !== undefined) stored.darwin = arraySlice(parsed.darwin);
            mapSet(this.#map, name, stored);
            return this;
        }

        get(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.get takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("executable name must be a string");
            }
            const entry = mapGet(this.#map, name);
            return entry === undefined ? undefined : clonePathFinderOptions(entry);
        }

        has(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.has takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("executable name must be a string");
            }
            return mapHas(this.#map, name);
        }

        delete(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.delete takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("executable name must be a string");
            }
            return mapDelete(this.#map, name);
        }

        clear() {
            if (arguments.length !== 0) {
                throw new makeTypeError("PathFinder.clear takes no arguments");
            }
            mapClear(this.#map);
        }

        #findRecord(name) {
            const exact = mapGet(this.#map, name);
            if (exact !== undefined) return exact;
            const lowerName = stringToLowerCase(name);
            let exactPropMatch = undefined;
            let foldMatch = undefined;
            mapForEach(this.#map, (entry, key) => {
                if (exactPropMatch !== undefined) return;
                if (entry.name === name || entry.cached === name) {
                    exactPropMatch = entry;
                    return;
                }
                if (
                    foldMatch === undefined &&
                    (stringToLowerCase(key) === lowerName ||
                        stringToLowerCase(entry.name) === lowerName)
                ) {
                    foldMatch = entry;
                }
            });
            return exactPropMatch !== undefined ? exactPropMatch : foldMatch;
        }

        find(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.find takes exactly one argument");
            }
            if (typeof name !== "string") {
                throw new makeTypeError("executable name must be a string");
            }
            const record = this.#findRecord(name);
            return record === undefined ? undefined : clonePathFinderOptions(record);
        }

        #ensureRecord(name) {
            validatePathFinderKey(name);
            let record = this.#findRecord(name);
            if (record === undefined) {
                const envBase = toUnderscoreUpper(name);
                record = objectCreate(null);
                record.name = name;
                if (envBase.length > 0) {
                    record.envVariable = envBase + "_EXE";
                }
                mapSet(this.#map, name, record);
            }
            return record;
        }

        findExeSync(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.findExeSync takes exactly one argument");
            }
            const record = this.#ensureRecord(name);

            if (record.envVariable !== undefined && record.envVariable.length > 0) {
                const rawEnv = hostEnvVar(record.envVariable);
                if (rawEnv !== null && rawEnv !== undefined && rawEnv.length > 0) {
                    const expanded = hostExpandEnvPath(rawEnv);
                    if (expanded !== null && expanded !== undefined && expanded.length > 0) {
                        if (record.noCache !== true && record.cached === expanded) {
                            return expanded;
                        }
                        if (hostIsExecutableFile(expanded)) {
                            if (record.noCache !== true) record.cached = expanded;
                            return expanded;
                        }
                    }
                }
            }

            if (
                record.executable !== undefined &&
                record.executable.length > 0 &&
                hasPathSeparator(record.executable) &&
                hostIsExecutableFile(record.executable)
            ) {
                if (record.noCache !== true) record.cached = record.executable;
                return record.executable;
            }

            if (record.noCache !== true && record.cached !== undefined && record.cached.length > 0) {
                return record.cached;
            }

            const target = record.executable !== undefined ? record.executable : name;
            const onPath = whichSync(target);
            if (onPath !== null && onPath !== undefined) {
                if (record.noCache !== true) record.cached = onPath;
                return onPath;
            }

            const os = hostPlatform();
            const lists = [];
            if (os === "windows") {
                if (record.windows !== undefined) pushOwn(lists, record.windows);
            } else if (os === "darwin") {
                if (record.darwin !== undefined) pushOwn(lists, record.darwin);
                if (record.linux !== undefined) pushOwn(lists, record.linux);
            } else {
                if (record.linux !== undefined) pushOwn(lists, record.linux);
            }
            for (let li = 0; li < lists.length; li++) {
                const candidates = lists[li];
                for (let ci = 0; ci < candidates.length; ci++) {
                    const expanded = hostExpandEnvPath(candidates[ci]);
                    if (
                        expanded !== null &&
                        expanded !== undefined &&
                        expanded.length > 0 &&
                        hostIsExecutableFile(expanded)
                    ) {
                        if (record.noCache !== true) record.cached = expanded;
                        return expanded;
                    }
                }
            }
            return undefined;
        }
    }
    freeze(PathFinder.prototype);
    freeze(PathFinder);

    const pathFinder = freeze(new PathFinder());

    class Command {
        #spec;
        #logHandler = null;
        #loggers = new makeMap();

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
                        "VAEL_EXEC_INVALID_ARGUMENT",
                        "program name must not be empty",
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
                if (parsedOpts.log !== undefined) {
                    this.#logHandler = parsedOpts.log;
                }
            }
        }

        #runLogHooks(resolvedFile, mode) {
            const handlers = [];
            appendActiveHandlers(globalLoggers, handlers);
            if (this.#logHandler !== null) {
                pushOwn(handlers, this.#logHandler);
            }
            appendActiveHandlers(this.#loggers, handlers);
            if (handlers.length === 0) return;

            const spec = this.#spec;
            const loggedArgs = freeze(arraySlice(spec.args));
            const event = objectCreate(null);
            defineOwn(event, "file", resolvedFile, true, false);
            defineOwn(event, "args", loggedArgs, true, false);
            if (spec.cwd !== undefined) {
                defineOwn(event, "cwd", spec.cwd, true, false);
            }
            defineOwn(event, "mode", mode, true, false);
            freeze(event);
            for (let i = 0; i < handlers.length; i++) {
                handlers[i](resolvedFile, loggedArgs, event);
            }
        }

        #prepareSync(mode) {
            const spec = this.#spec;
            let resolvedFile = spec.program;
            if (pathFinder.find(spec.program) !== undefined) {
                const found = pathFinder.findExeSync(spec.program);
                if (found === undefined) {
                    throw new NotFoundOnPathError(spec.program);
                }
                resolvedFile = found;
            }
            this.#runLogHooks(resolvedFile, mode);
            if (resolvedFile === spec.program) {
                return spec;
            }
            const copy = objectCreate(null);
            defineOwn(copy, "program", resolvedFile, true, true);
            defineOwn(copy, "args", spec.args, true, true);
            defineOwn(copy, "cwd", spec.cwd, true, true);
            defineOwn(copy, "clearEnv", spec.clearEnv, true, true);
            defineOwn(copy, "envOps", spec.envOps, true, true);
            defineOwn(copy, "stdin", spec.stdin, true, true);
            defineOwn(copy, "stdout", spec.stdout, true, true);
            defineOwn(copy, "stderr", spec.stderr, true, true);
            defineOwn(copy, "timeoutMs", spec.timeoutMs, true, true);
            defineOwn(copy, "maxOutputBytes", spec.maxOutputBytes, true, true);
            defineOwn(copy, "killSignal", spec.killSignal, true, true);
            defineOwn(copy, "killGraceMs", spec.killGraceMs, true, true);
            return copy;
        }

        log(handler) {
            if (arguments.length !== 1) {
                throw new makeTypeError("log takes exactly one argument");
            }
            if (handler !== null && typeof handler !== "function") {
                throw new makeTypeError("log handler must be a function or null");
            }
            this.#logHandler = handler;
            return this;
        }

        addLogger(handler) {
            if (arguments.length !== 1) {
                throw new makeTypeError("addLogger takes exactly one argument");
            }
            return createLogSubscription(this.#loggers, handler);
        }

        clearLoggers() {
            if (arguments.length !== 0) {
                throw new makeTypeError("clearLoggers takes no arguments");
            }
            this.#logHandler = null;
            clearLogRegistry(this.#loggers);
            return this;
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
                    "VAEL_EXEC_INVALID_ARGUMENT",
                    "working directory must not be empty or contain NUL",
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
                    "VAEL_EXEC_INVALID_ARGUMENT",
                    "environment variable value must not contain a NUL byte",
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
                    "VAEL_EXEC_INVALID_ARGUMENT",
                    "maxOutputBytes exceeds hard limit of 64 MiB",
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
                    "VAEL_EXEC_INVALID_ARGUMENT",
                    "killGrace must be a non-negative finite number",
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
            const spec = this.#prepareSync("execSync");
            const res = hostExecSync(spec);
            if (!res.ok) {
                throwExecFailure(res, spec.program);
            }
            return new Output(INTERNAL_TOKEN, res);
        }

        execSync() {
            return this.outputSync();
        }

        statusSync() {
            const spec = this.#prepareSync("runSync");
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
    freeze(Command.prototype);
    freeze(Command);

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
    freeze(FileLock.prototype);
    freeze(FileLock);

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

    const mod = objectCreate(null);
    defineOwn(mod, "Command", Command, true, false);
    defineOwn(mod, "CommandError", CommandError, true, false);
    defineOwn(mod, "FileLock", FileLock, true, false);
    defineOwn(mod, "NotFoundOnPathError", NotFoundOnPathError, true, false);
    defineOwn(mod, "Output", Output, true, false);
    defineOwn(mod, "PathFinder", PathFinder, true, false);
    defineOwn(mod, "addLogger", addLogger, true, false);
    defineOwn(mod, "clearLoggers", clearLoggers, true, false);
    defineOwn(mod, "cmd", cmd, true, false);
    defineOwn(mod, "execSync", execSync, true, false);
    defineOwn(mod, "lockFileSync", lockFileSync, true, false);
    defineOwn(mod, "pathFinder", pathFinder, true, false);
    defineOwn(mod, "runSync", runSync, true, false);
    defineOwn(mod, "whichAllSync", whichAllSync, true, false);
    defineOwn(mod, "whichSync", whichSync, true, false);
    return freeze(mod);
})
