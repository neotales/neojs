// The `vael:exec` built-in module factory, evaluated inside `vael.js` only when
// the `vael-exec` Cargo feature is enabled.
(function createVaelExec(deps) {
    "use strict";

    const ops = deps.ops;
    const opValidateProgram = ops.op_vael_exec_validate_program;
    const opValidateArg = ops.op_vael_exec_validate_arg;
    const opValidateArgs = ops.op_vael_exec_validate_args;
    const opValidateCwd = ops.op_vael_exec_validate_cwd;
    const opValidateEnvEntry = ops.op_vael_exec_validate_env_entry;
    const opValidateEnvs = ops.op_vael_exec_validate_envs;
    const opValidateEnvRemove = ops.op_vael_exec_validate_env_remove;
    const opValidateStdin = ops.op_vael_exec_validate_stdin;
    const opValidateStdio = ops.op_vael_exec_validate_stdio;
    const opValidateTimeout = ops.op_vael_exec_validate_timeout;
    const opValidateMaxOutput = ops.op_vael_exec_validate_max_output;
    const opValidateSignal = ops.op_vael_exec_validate_signal;
    const opValidateKillGrace = ops.op_vael_exec_validate_kill_grace;
    const opParseCommandOptions = ops.op_vael_exec_parse_command_options;
    const opParsePathFinderOptions = ops.op_vael_exec_parse_path_finder_options;
    const opIsExecutableFile = ops.op_vael_exec_is_executable_file;
    const opEnvVar = ops.op_vael_exec_env_var;
    const opExpandEnvPath = ops.op_vael_exec_expand_env_path;
    const opPlatform = ops.op_vael_exec_platform;
    const opOutputSync = ops.op_vael_exec_output_sync;
    const opOutputAsync = ops.op_vael_exec_output_async;
    const opStatusSync = ops.op_vael_exec_status_sync;
    const opStatusAsync = ops.op_vael_exec_status_async;
    const opDecodeUtf8Lossy = ops.op_vael_exec_decode_utf8_lossy;
    const opSpawn = ops.op_vael_exec_spawn;
    const opChildWriteStdin = ops.op_vael_exec_child_write_stdin;
    const opChildCloseStdin = ops.op_vael_exec_child_close_stdin;
    const opChildReadStdout = ops.op_vael_exec_child_read_stdout;
    const opChildReadStderr = ops.op_vael_exec_child_read_stderr;
    const opChildKill = ops.op_vael_exec_child_kill;
    const opChildTryWait = ops.op_vael_exec_child_try_wait;
    const opChildWait = ops.op_vael_exec_child_wait;
    const opChildOutput = ops.op_vael_exec_child_output;
    const opChildDispose = ops.op_vael_exec_child_dispose;
    const opWhichSync = ops.op_vael_exec_which_sync;
    const opWhichAsync = ops.op_vael_exec_which_async;
    const opWhichAllSync = ops.op_vael_exec_which_all_sync;
    const opWhichAllAsync = ops.op_vael_exec_which_all_async;
    const opLockSync = ops.op_vael_exec_lock_sync;
    const opLockAsync = ops.op_vael_exec_lock_async;
    const opLockUnlock = ops.op_vael_exec_lock_unlock;

    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRegExp = RegExp;
    const makeUint8Array = Uint8Array;
    const freeze = Object.freeze;
    const defineProperty = Object.defineProperty;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const getPrototypeOf = Object.getPrototypeOf;
    const makeMap = Map;
    const mapGet = Function.prototype.call.bind(Map.prototype.get);
    const mapSet = Function.prototype.call.bind(Map.prototype.set);
    const mapHas = Function.prototype.call.bind(Map.prototype.has);
    const mapDelete = Function.prototype.call.bind(Map.prototype.delete);
    const mapClear = Function.prototype.call.bind(Map.prototype.clear);
    const mapForEach = Function.prototype.call.bind(Map.prototype.forEach);
    const hasOwn = Object.hasOwn;
    const arrayIsArray = Array.isArray;
    const arraySlice = Function.prototype.call.bind(Array.prototype.slice);
    const arrayPush = Function.prototype.call.bind(Array.prototype.push);
    const arrayPop = Function.prototype.call.bind(Array.prototype.pop);
    const stringSplit = Function.prototype.call.bind(String.prototype.split);
    const stringIncludes = Function.prototype.call.bind(String.prototype.includes);
    const stringReplace = Function.prototype.call.bind(String.prototype.replace);
    const stringToLowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    const stringToUpperCase = Function.prototype.call.bind(String.prototype.toUpperCase);
    const jsonParse = JSON.parse;

    const CAMEL_BOUNDARY_RE = makeRegExp("([a-z0-9])([A-Z])", "g");
    const NON_ALNUM_RE = makeRegExp("[^a-zA-Z0-9]+", "g");
    const EDGE_UNDERSCORE_RE = makeRegExp("^_+|_+$", "g");

    const typedArrayProto = getPrototypeOf(Uint8Array.prototype);
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);
    const typedArrayIndexOf = Function.prototype.call.bind(typedArrayProto.indexOf);
    const typedArrayByteLength = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayProto, "byteLength").get,
    );

    const disposeSymbol = typeof Symbol.dispose === "symbol" ? Symbol.dispose : null;
    const asyncDisposeSymbol =
        typeof Symbol.asyncDispose === "symbol" ? Symbol.asyncDispose : null;

    const LINE_SPLIT_RE = makeRegExp("\\r?\\n");
    const outputToken = Symbol("vaeljs.exec.Output");
    const childToken = Symbol("vaeljs.exec.ChildProcess");
    const lockToken = Symbol("vaeljs.exec.FileLock");

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

    function splitLines(text) {
        if (text.length === 0) return [];
        const lines = stringSplit(text, LINE_SPLIT_RE);
        if (lines.length > 0 && lines[lines.length - 1] === "") {
            arrayPop(lines);
        }
        return lines;
    }

    /**
     * Error thrown by `vael:exec` for process execution, validation, signal,
     * timeout, output limit, or file-locking failures.
     */
    class CommandError extends makeError {
        constructor(code, message, options) {
            super(message === undefined ? "" : message);
            defineOwn(this, "name", "CommandError", false, true);
            defineOwn(this, "code", code === undefined ? "VAEL_EXEC_IO" : code, true, false);
            const hasOpts = options !== undefined && options !== null;
            const timedOut =
                hasOpts && hasOwn(options, "timedOut") && options.timedOut === true;
            defineOwn(this, "timedOut", timedOut, true, false);
            if (hasOpts) {
                if (
                    hasOwn(options, "exitCode") &&
                    options.exitCode !== undefined &&
                    options.exitCode !== null
                ) {
                    defineOwn(this, "exitCode", options.exitCode, true, false);
                }
                if (hasOwn(options, "signal") && options.signal !== undefined) {
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

    /**
     * Error thrown when a target executable cannot be found on `PATH` or at an
     * explicit executable path.
     */
    class NotFoundOnPathError extends makeError {
        constructor(fileName, message) {
            super(message !== undefined ? message : `executable not found: ${fileName}`);
            defineOwn(this, "name", "NotFoundOnPathError", false, true);
            defineOwn(this, "code", "VAEL_EXEC_NOT_FOUND", true, false);
            defineOwn(this, "fileName", fileName, true, false);
        }
    }
    freeze(NotFoundOnPathError.prototype);
    freeze(NotFoundOnPathError);

    function throwExec(outcome) {
        if (outcome.name === "TypeError") {
            throw new makeTypeError(outcome.message);
        }
        if (outcome.name === "NotFoundOnPathError") {
            throw new NotFoundOnPathError(outcome.fileName, outcome.message);
        }
        throw new CommandError(outcome.code, outcome.message, {
            __proto__: null,
            exitCode: outcome.exitCode,
            signal: outcome.signal,
            fileName: outcome.fileName,
        });
    }

    function unwrapExec(outcome) {
        if (!outcome.ok) throwExec(outcome);
        return outcome.value;
    }

    function freezeStatus(raw) {
        return freeze({
            __proto__: null,
            code: raw.code,
            signal: raw.signal,
            success: raw.success,
            timedOut: raw.timedOut,
        });
    }

    /**
     * Captured standard output, standard error, and exit metadata from a
     * completed command.
     */
    class Output {
        #code;
        #signal;
        #success;
        #stdout;
        #stderr;
        #timedOut;
        #truncated;
        #fileName;

        constructor(token, raw, fileName) {
            if (token !== outputToken) {
                throw new makeTypeError("Output cannot be constructed directly");
            }
            this.#code = raw.code;
            this.#signal = raw.signal;
            this.#success = raw.success;
            this.#stdout = raw.stdout;
            this.#stderr = raw.stderr;
            this.#timedOut = raw.timedOut;
            this.#truncated = raw.truncated;
            this.#fileName = fileName;
            freeze(this);
        }

        get code() {
            return this.#code;
        }

        get signal() {
            return this.#signal;
        }

        get success() {
            return this.#success;
        }

        get stdout() {
            return copyBytes(this.#stdout);
        }

        get stderr() {
            return copyBytes(this.#stderr);
        }

        get timedOut() {
            return this.#timedOut;
        }

        get truncated() {
            return this.#truncated;
        }

        /** Decode captured `stdout` as UTF-8 (lossy on invalid sequences). */
        text() {
            if (arguments.length !== 0) {
                throw new makeTypeError("text takes no arguments");
            }
            return unwrapExec(opDecodeUtf8Lossy(this.#stdout));
        }

        /** Split decoded `stdout` into lines. */
        lines() {
            if (arguments.length !== 0) {
                throw new makeTypeError("lines takes no arguments");
            }
            return splitLines(this.text());
        }

        /** Parse decoded `stdout` as JSON. */
        json() {
            if (arguments.length !== 0) {
                throw new makeTypeError("json takes no arguments");
            }
            return jsonParse(this.text());
        }

        /** Decode captured `stderr` as UTF-8 (lossy on invalid sequences). */
        errorText() {
            if (arguments.length !== 0) {
                throw new makeTypeError("errorText takes no arguments");
            }
            return unwrapExec(opDecodeUtf8Lossy(this.#stderr));
        }

        /** Split decoded `stderr` into lines. */
        errorLines() {
            if (arguments.length !== 0) {
                throw new makeTypeError("errorLines takes no arguments");
            }
            return splitLines(this.errorText());
        }

        /** Parse decoded `stderr` as JSON. */
        errorJson() {
            if (arguments.length !== 0) {
                throw new makeTypeError("errorJson takes no arguments");
            }
            return jsonParse(this.errorText());
        }

        /**
         * Validate the command outcome, throwing `CommandError` on failure.
         *
         * @param {(code: number) => boolean} [predicate]
         * @param {boolean} [failOnStderr=false]
         * @returns {Output} `this`
         */
        validate(predicate, failOnStderr) {
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
            const ok =
                predicate !== undefined && predicate !== null
                    ? Boolean(predicate(this.#code))
                    : this.#success;
            if (!ok) {
                const code = this.#timedOut ? "VAEL_EXEC_TIMEOUT" : "VAEL_EXEC_NON_ZERO";
                const msg = this.#timedOut
                    ? `command timed out (exit code ${this.#code})`
                    : `command failed with exit code ${this.#code}`;
                throw new CommandError(code, msg, {
                    __proto__: null,
                    exitCode: this.#code,
                    signal: this.#signal,
                    timedOut: this.#timedOut,
                    output: this,
                    fileName: this.#fileName,
                });
            }
            if (failOnStderr === true && typedArrayByteLength(this.#stderr) > 0) {
                throw new CommandError("VAEL_EXEC_STDERR", "command produced non-empty stderr", {
                    __proto__: null,
                    exitCode: this.#code,
                    signal: this.#signal,
                    timedOut: this.#timedOut,
                    output: this,
                    fileName: this.#fileName,
                });
            }
            return this;
        }
    }
    freeze(Output.prototype);
    freeze(Output);

    const childDisposers = new WeakMap();
    const weakMapSet = Function.prototype.call.bind(WeakMap.prototype.set);
    const weakMapGet = Function.prototype.call.bind(WeakMap.prototype.get);

    function disposedChildError(fileName) {
        return new CommandError("VAEL_EXEC_INVALID_ARGUMENT", "child process handle is disposed", {
            __proto__: null,
            fileName,
        });
    }

    /**
     * Managed spawned child process with streaming stdin/stdout/stderr, signal
     * delivery, waiting, and deterministic `[Symbol.asyncDispose]()` cleanup.
     */
    class ChildProcess {
        #id;
        #pid;
        #fileName;
        #disposed = false;

        constructor(token, id, pid, fileName) {
            if (token !== childToken) {
                throw new makeTypeError(
                    "ChildProcess cannot be constructed directly; use spawn()",
                );
            }
            this.#id = id;
            this.#pid = pid;
            this.#fileName = fileName;
            weakMapSet(childDisposers, this, async () => {
                if (this.#disposed) return;
                this.#disposed = true;
                const res = await opChildDispose(this.#id);
                if (!res.ok) throwExec(res);
            });
            freeze(this);
        }

        get pid() {
            return this.#pid;
        }

        async writeStdin(data) {
            if (arguments.length !== 1) {
                throw new makeTypeError("writeStdin takes exactly one argument");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            const res = await opChildWriteStdin(this.#id, data);
            if (!res.ok) throwExec(res);
        }

        closeStdin() {
            if (arguments.length !== 0) {
                throw new makeTypeError("closeStdin takes no arguments");
            }
            if (this.#disposed) return;
            const res = opChildCloseStdin(this.#id);
            if (!res.ok) throwExec(res);
        }

        async readStdout(maxBytes) {
            if (arguments.length > 1) {
                throw new makeTypeError("readStdout takes zero or one argument");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            return unwrapExec(await opChildReadStdout(this.#id, maxBytes));
        }

        async readStderr(maxBytes) {
            if (arguments.length > 1) {
                throw new makeTypeError("readStderr takes zero or one argument");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            return unwrapExec(await opChildReadStderr(this.#id, maxBytes));
        }

        kill(signal) {
            if (arguments.length > 1) {
                throw new makeTypeError("kill takes zero or one argument");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            const res = opChildKill(this.#id, signal);
            if (!res.ok) throwExec(res);
        }

        tryWait() {
            if (arguments.length !== 0) {
                throw new makeTypeError("tryWait takes no arguments");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            const status = unwrapExec(opChildTryWait(this.#id));
            return status === null ? null : freezeStatus(status);
        }

        async wait() {
            if (arguments.length !== 0) {
                throw new makeTypeError("wait takes no arguments");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            return freezeStatus(unwrapExec(await opChildWait(this.#id)));
        }

        async waitWithOutput(options) {
            if (arguments.length > 1) {
                throw new makeTypeError("waitWithOutput takes zero or one argument");
            }
            if (this.#disposed) throw disposedChildError(this.#fileName);
            const raw = unwrapExec(await opChildOutput(this.#id, options));
            return new Output(outputToken, raw, this.#fileName);
        }

        async output() {
            if (arguments.length !== 0) {
                throw new makeTypeError("output takes no arguments");
            }
            return await this.waitWithOutput();
        }
    }

    if (asyncDisposeSymbol !== null) {
        defineProperty(ChildProcess.prototype, asyncDisposeSymbol, {
            __proto__: null,
            value: async function () {
                const fn = weakMapGet(childDisposers, this);
                if (fn !== undefined) {
                    await fn();
                }
            },
            enumerable: false,
            writable: false,
            configurable: false,
        });
    }
    freeze(ChildProcess.prototype);
    freeze(ChildProcess);

    /**
     * Active OS advisory file lock guard with deterministic `unlock()`,
     * `[Symbol.dispose]()`, and `[Symbol.asyncDispose]()` cleanup.
     */
    class FileLock {
        #id;
        #path;
        #mode;
        #locked = true;

        constructor(token, id, path, mode) {
            if (token !== lockToken) {
                throw new makeTypeError(
                    "FileLock cannot be constructed directly; use lockFile() or lockFileSync()",
                );
            }
            this.#id = id;
            this.#path = path;
            this.#mode = mode;
            freeze(this);
        }

        get path() {
            return this.#path;
        }

        get mode() {
            return this.#mode;
        }

        get locked() {
            return this.#locked;
        }

        unlock() {
            if (arguments.length !== 0) {
                throw new makeTypeError("unlock takes no arguments");
            }
            if (!this.#locked) return;
            this.#locked = false;
            const res = opLockUnlock(this.#id);
            if (!res.ok) throwExec(res);
        }
    }

    if (disposeSymbol !== null) {
        defineProperty(FileLock.prototype, disposeSymbol, {
            __proto__: null,
            value: function () {
                this.unlock();
            },
            enumerable: false,
            writable: false,
            configurable: false,
        });
    }
    if (asyncDisposeSymbol !== null) {
        defineProperty(FileLock.prototype, asyncDisposeSymbol, {
            __proto__: null,
            value: async function () {
                this.unlock();
            },
            enumerable: false,
            writable: false,
            configurable: false,
        });
    }
    freeze(FileLock.prototype);
    freeze(FileLock);

    const globalLoggers = new makeMap();

    function createLogSubscription(registry, handler) {
        if (typeof handler !== "function") {
            throw new makeTypeError("log handler must be a function");
        }
        const token = { __proto__: null };
        const entry = { __proto__: null, handler, active: true };
        mapSet(registry, token, entry);
        const dispose = () => {
            if (entry.active) {
                entry.active = false;
                mapDelete(registry, token);
            }
        };
        const sub = { __proto__: null };
        defineProperty(sub, "active", {
            __proto__: null,
            get: () => entry.active,
            enumerable: true,
            configurable: true,
        });
        defineOwn(sub, "dispose", dispose, true, false);
        if (disposeSymbol !== null) {
            defineOwn(sub, disposeSymbol, dispose, false, false);
        }
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
                arrayPush(out, entry.handler);
            }
        });
    }

    /**
     * Register a global command execution log/audit handler.
     *
     * @param {(file: string, args: readonly string[], event: CommandLogEvent) => void} handler
     * @returns {CommandLogSubscription}
     */
    function addLogger(handler) {
        if (arguments.length !== 1) {
            throw new makeTypeError("addLogger takes exactly one argument");
        }
        return createLogSubscription(globalLoggers, handler);
    }

    /**
     * Remove all global command execution log handlers and mark their
     * subscriptions inactive.
     */
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

    /**
     * Registry for resolving and caching platform executable paths.
     */
    class PathFinder {
        #map = new makeMap();

        set(name, options) {
            if (arguments.length !== 2) {
                throw new makeTypeError("PathFinder.set takes exactly two arguments");
            }
            validatePathFinderKey(name);
            const parsed = unwrapExec(opParsePathFinderOptions(options));
            const stored = { __proto__: null, name: parsed.name };
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
                record = { __proto__: null, name };
                if (envBase.length > 0) {
                    record.envVariable = envBase + "_EXE";
                }
                mapSet(this.#map, name, record);
            }
            return record;
        }

        #resolveEnvOrCached(record) {
            if (record.envVariable !== undefined && record.envVariable.length > 0) {
                const rawEnv = opEnvVar(record.envVariable);
                if (rawEnv !== null && rawEnv !== undefined && rawEnv.length > 0) {
                    const expanded = opExpandEnvPath(rawEnv);
                    if (expanded !== null && expanded !== undefined && expanded.length > 0) {
                        if (record.noCache !== true && record.cached === expanded) {
                            return expanded;
                        }
                        if (opIsExecutableFile(expanded)) {
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
                opIsExecutableFile(record.executable)
            ) {
                if (record.noCache !== true) record.cached = record.executable;
                return record.executable;
            }
            if (record.noCache !== true && record.cached !== undefined && record.cached.length > 0) {
                return record.cached;
            }
            return undefined;
        }

        #resolveFallback(record) {
            const os = opPlatform();
            const lists = [];
            if (os === "windows") {
                if (record.windows !== undefined) arrayPush(lists, record.windows);
            } else if (os === "darwin") {
                if (record.darwin !== undefined) arrayPush(lists, record.darwin);
                if (record.linux !== undefined) arrayPush(lists, record.linux);
            } else {
                if (record.linux !== undefined) arrayPush(lists, record.linux);
            }
            for (let li = 0; li < lists.length; li += 1) {
                const candidates = lists[li];
                for (let ci = 0; ci < candidates.length; ci += 1) {
                    const expanded = opExpandEnvPath(candidates[ci]);
                    if (
                        expanded !== null &&
                        expanded !== undefined &&
                        expanded.length > 0 &&
                        opIsExecutableFile(expanded)
                    ) {
                        if (record.noCache !== true) record.cached = expanded;
                        return expanded;
                    }
                }
            }
            return undefined;
        }

        findExeSync(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.findExeSync takes exactly one argument");
            }
            const record = this.#ensureRecord(name);
            const early = this.#resolveEnvOrCached(record);
            if (early !== undefined) return early;

            const target = record.executable !== undefined ? record.executable : name;
            const onPath = whichSync(target);
            if (onPath !== null && onPath !== undefined) {
                if (record.noCache !== true) record.cached = onPath;
                return onPath;
            }
            return this.#resolveFallback(record);
        }

        async findExe(name) {
            if (arguments.length !== 1) {
                throw new makeTypeError("PathFinder.findExe takes exactly one argument");
            }
            const record = this.#ensureRecord(name);
            const early = this.#resolveEnvOrCached(record);
            if (early !== undefined) return early;

            const target = record.executable !== undefined ? record.executable : name;
            const onPath = await whichFn(target);
            if (onPath !== null && onPath !== undefined) {
                if (record.noCache !== true) record.cached = onPath;
                return onPath;
            }
            return this.#resolveFallback(record);
        }
    }
    freeze(PathFinder.prototype);
    freeze(PathFinder);

    const pathFinder = freeze(new PathFinder());

    /**
     * Argv-only command builder that never invokes a shell.
     */
    class Command {
        #program;
        #args = [];
        #cwd;
        #clearEnv = false;
        #envOps = [];
        #stdin;
        #stdout;
        #stderr;
        #timeoutMs;
        #killSignal;
        #killGraceMs;
        #maxOutputBytes;
        #logHandler = null;
        #loggers = new makeMap();

        constructor(programOrArgv, argsOrOptions, maybeOptions) {
            if (arguments.length < 1 || arguments.length > 3) {
                throw new makeTypeError("Command takes one to three arguments");
            }
            let program;
            let initialArgs;
            let options;

            if (arrayIsArray(programOrArgv)) {
                if (arguments.length > 2) {
                    throw new makeTypeError(
                        "Command(argv, options?) takes at most two arguments",
                    );
                }
                const validatedArgv = unwrapExec(opValidateArgs(programOrArgv));
                if (validatedArgv.length === 0) {
                    throw new CommandError(
                        "VAEL_EXEC_INVALID_ARGUMENT",
                        "program name must not be empty",
                    );
                }
                program = unwrapExec(opValidateProgram(validatedArgv[0]));
                initialArgs = arraySlice(validatedArgv, 1);
                options = argsOrOptions;
            } else {
                program = unwrapExec(opValidateProgram(programOrArgv));
                if (argsOrOptions === undefined) {
                    initialArgs = [];
                    options = maybeOptions;
                } else if (arrayIsArray(argsOrOptions)) {
                    initialArgs = unwrapExec(opValidateArgs(argsOrOptions));
                    options = maybeOptions;
                } else if (arguments.length === 2) {
                    initialArgs = [];
                    options = argsOrOptions;
                } else {
                    throw new makeTypeError("args must be an array of strings");
                }
            }

            const parsedOpts = unwrapExec(opParseCommandOptions(options));
            this.#program = program;
            for (let i = 0; i < initialArgs.length; i += 1) {
                arrayPush(this.#args, initialArgs[i]);
            }
            if (parsedOpts.args !== undefined) {
                for (let i = 0; i < parsedOpts.args.length; i += 1) {
                    arrayPush(this.#args, parsedOpts.args[i]);
                }
            }
            if (parsedOpts.cwd !== undefined) this.#cwd = parsedOpts.cwd;
            if (parsedOpts.clearEnv === true) this.#clearEnv = true;
            if (parsedOpts.env !== undefined) {
                for (let i = 0; i < parsedOpts.env.length; i += 1) {
                    const pair = parsedOpts.env[i];
                    arrayPush(this.#envOps, ["set", pair[0], pair[1]]);
                }
            }
            if (parsedOpts.envRemove !== undefined) {
                for (let i = 0; i < parsedOpts.envRemove.length; i += 1) {
                    arrayPush(this.#envOps, ["remove", parsedOpts.envRemove[i]]);
                }
            }
            if (parsedOpts.stdin !== undefined) this.#stdin = parsedOpts.stdin;
            if (parsedOpts.stdout !== undefined) this.#stdout = parsedOpts.stdout;
            if (parsedOpts.stderr !== undefined) this.#stderr = parsedOpts.stderr;
            if (parsedOpts.timeoutMs !== undefined) this.#timeoutMs = parsedOpts.timeoutMs;
            if (parsedOpts.killSignal !== undefined) this.#killSignal = parsedOpts.killSignal;
            if (parsedOpts.killGraceMs !== undefined) this.#killGraceMs = parsedOpts.killGraceMs;
            if (parsedOpts.maxOutputBytes !== undefined) {
                this.#maxOutputBytes = parsedOpts.maxOutputBytes;
            }
            if (parsedOpts.hasLog === true) {
                this.#logHandler = options.log;
            }
        }

        #specObject(programOverride) {
            const spec = {
                __proto__: null,
                program: programOverride !== undefined ? programOverride : this.#program,
                args: arraySlice(this.#args),
                clearEnv: this.#clearEnv,
                envOps: arraySlice(this.#envOps),
            };
            if (this.#cwd !== undefined) spec.cwd = this.#cwd;
            if (this.#stdin !== undefined) spec.stdin = this.#stdin;
            if (this.#stdout !== undefined) spec.stdout = this.#stdout;
            if (this.#stderr !== undefined) spec.stderr = this.#stderr;
            if (this.#timeoutMs !== undefined) spec.timeoutMs = this.#timeoutMs;
            if (this.#killSignal !== undefined) spec.killSignal = this.#killSignal;
            if (this.#killGraceMs !== undefined) spec.killGraceMs = this.#killGraceMs;
            if (this.#maxOutputBytes !== undefined) spec.maxOutputBytes = this.#maxOutputBytes;
            return spec;
        }

        #runLogHooks(resolvedFile, mode) {
            const handlers = [];
            appendActiveHandlers(globalLoggers, handlers);
            if (this.#logHandler !== null) {
                arrayPush(handlers, this.#logHandler);
            }
            appendActiveHandlers(this.#loggers, handlers);
            if (handlers.length === 0) return;

            const loggedArgs = freeze(arraySlice(this.#args));
            const event = {
                __proto__: null,
                file: resolvedFile,
                args: loggedArgs,
                mode,
            };
            if (this.#cwd !== undefined) {
                event.cwd = this.#cwd;
            }
            freeze(event);
            for (let i = 0; i < handlers.length; i += 1) {
                handlers[i](resolvedFile, loggedArgs, event);
            }
        }

        #prepareSync(mode) {
            let resolvedFile = this.#program;
            if (pathFinder.find(this.#program) !== undefined) {
                const found = pathFinder.findExeSync(this.#program);
                if (found === undefined) {
                    throw new NotFoundOnPathError(this.#program);
                }
                resolvedFile = found;
            }
            this.#runLogHooks(resolvedFile, mode);
            return this.#specObject(resolvedFile);
        }

        async #prepareAsync(mode) {
            let resolvedFile = this.#program;
            if (pathFinder.find(this.#program) !== undefined) {
                const found = await pathFinder.findExe(this.#program);
                if (found === undefined) {
                    throw new NotFoundOnPathError(this.#program);
                }
                resolvedFile = found;
            }
            this.#runLogHooks(resolvedFile, mode);
            return this.#specObject(resolvedFile);
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
            if (arguments.length !== 1) {
                throw new makeTypeError("arg takes exactly one argument");
            }
            const res = opValidateArg(v);
            if (!res.ok) throwExec(res);
            arrayPush(this.#args, v);
            return this;
        }

        args(vs) {
            if (arguments.length !== 1) {
                throw new makeTypeError("args takes exactly one argument");
            }
            const validated = unwrapExec(opValidateArgs(vs));
            for (let i = 0; i < validated.length; i += 1) {
                arrayPush(this.#args, validated[i]);
            }
            return this;
        }

        cwd(dir) {
            if (arguments.length !== 1) {
                throw new makeTypeError("cwd takes exactly one argument");
            }
            const res = opValidateCwd(dir);
            if (!res.ok) throwExec(res);
            this.#cwd = dir;
            return this;
        }

        env(k, v) {
            if (arguments.length !== 2) {
                throw new makeTypeError("env takes exactly two arguments");
            }
            const res = opValidateEnvEntry(k, v);
            if (!res.ok) throwExec(res);
            arrayPush(this.#envOps, ["set", k, v]);
            return this;
        }

        envs(obj) {
            if (arguments.length !== 1) {
                throw new makeTypeError("envs takes exactly one argument");
            }
            const pairs = unwrapExec(opValidateEnvs(obj));
            for (let i = 0; i < pairs.length; i += 1) {
                arrayPush(this.#envOps, ["set", pairs[i][0], pairs[i][1]]);
            }
            return this;
        }

        envRemove(k) {
            if (arguments.length !== 1) {
                throw new makeTypeError("envRemove takes exactly one argument");
            }
            const res = opValidateEnvRemove(k);
            if (!res.ok) throwExec(res);
            arrayPush(this.#envOps, ["remove", k]);
            return this;
        }

        envClear() {
            if (arguments.length !== 0) {
                throw new makeTypeError("envClear takes no arguments");
            }
            this.#clearEnv = true;
            this.#envOps = [];
            return this;
        }

        stdin(modeOrData) {
            if (arguments.length !== 1) {
                throw new makeTypeError("stdin takes exactly one argument");
            }
            this.#stdin = unwrapExec(opValidateStdin(modeOrData));
            return this;
        }

        stdout(mode) {
            if (arguments.length !== 1) {
                throw new makeTypeError("stdout takes exactly one argument");
            }
            this.#stdout = unwrapExec(opValidateStdio(mode));
            return this;
        }

        stderr(mode) {
            if (arguments.length !== 1) {
                throw new makeTypeError("stderr takes exactly one argument");
            }
            this.#stderr = unwrapExec(opValidateStdio(mode));
            return this;
        }

        timeout(ms) {
            if (arguments.length !== 1) {
                throw new makeTypeError("timeout takes exactly one argument");
            }
            this.#timeoutMs = unwrapExec(opValidateTimeout(ms));
            return this;
        }

        maxOutputBytes(n) {
            if (arguments.length !== 1) {
                throw new makeTypeError("maxOutputBytes takes exactly one argument");
            }
            this.#maxOutputBytes = unwrapExec(opValidateMaxOutput(n));
            return this;
        }

        killSignal(sig) {
            if (arguments.length !== 1) {
                throw new makeTypeError("killSignal takes exactly one argument");
            }
            this.#killSignal = unwrapExec(opValidateSignal(sig));
            return this;
        }

        killGrace(ms) {
            if (arguments.length !== 1) {
                throw new makeTypeError("killGrace takes exactly one argument");
            }
            this.#killGraceMs = unwrapExec(opValidateKillGrace(ms));
            return this;
        }

        toArgs() {
            if (arguments.length !== 0) {
                throw new makeTypeError("toArgs takes no arguments");
            }
            const out = [];
            defineOwn(out, 0, this.#program, true, true);
            for (let i = 0; i < this.#args.length; i += 1) {
                defineOwn(out, i + 1, this.#args[i], true, true);
            }
            return out;
        }

        outputSync() {
            if (arguments.length !== 0) {
                throw new makeTypeError("outputSync takes no arguments");
            }
            const spec = this.#prepareSync("execSync");
            const raw = unwrapExec(opOutputSync(spec));
            return new Output(outputToken, raw, spec.program);
        }

        execSync() {
            if (arguments.length !== 0) {
                throw new makeTypeError("execSync takes no arguments");
            }
            return this.outputSync();
        }

        statusSync() {
            if (arguments.length !== 0) {
                throw new makeTypeError("statusSync takes no arguments");
            }
            const spec = this.#prepareSync("runSync");
            return freezeStatus(unwrapExec(opStatusSync(spec)));
        }

        runSync() {
            if (arguments.length !== 0) {
                throw new makeTypeError("runSync takes no arguments");
            }
            return this.statusSync();
        }

        async output() {
            if (arguments.length !== 0) {
                throw new makeTypeError("output takes no arguments");
            }
            const spec = await this.#prepareAsync("exec");
            const raw = unwrapExec(await opOutputAsync(spec));
            return new Output(outputToken, raw, spec.program);
        }

        async exec() {
            if (arguments.length !== 0) {
                throw new makeTypeError("exec takes no arguments");
            }
            return await this.output();
        }

        async status() {
            if (arguments.length !== 0) {
                throw new makeTypeError("status takes no arguments");
            }
            const spec = await this.#prepareAsync("run");
            return freezeStatus(unwrapExec(await opStatusAsync(spec)));
        }

        async run() {
            if (arguments.length !== 0) {
                throw new makeTypeError("run takes no arguments");
            }
            return await this.status();
        }

        spawn() {
            if (arguments.length !== 0) {
                throw new makeTypeError("spawn takes no arguments");
            }
            const spec = this.#prepareSync("spawn");
            const res = opSpawn(spec);
            if (!res.ok) throwExec(res);
            return new ChildProcess(childToken, res.id, res.pid, spec.program);
        }

        async text() {
            if (arguments.length !== 0) {
                throw new makeTypeError("text takes no arguments");
            }
            return (await this.output()).validate().text();
        }

        async lines() {
            if (arguments.length !== 0) {
                throw new makeTypeError("lines takes no arguments");
            }
            return (await this.output()).validate().lines();
        }

        async json() {
            if (arguments.length !== 0) {
                throw new makeTypeError("json takes no arguments");
            }
            return (await this.output()).validate().json();
        }
    }
    freeze(Command.prototype);
    freeze(Command);

    function cmd(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1) return new Command(programOrArgv);
        if (arguments.length === 2) return new Command(programOrArgv, argsOrOptions);
        return new Command(programOrArgv, argsOrOptions, maybeOptions);
    }

    async function exec(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return await programOrArgv.output();
        }
        if (arguments.length === 1) return await new Command(programOrArgv).output();
        if (arguments.length === 2) {
            return await new Command(programOrArgv, argsOrOptions).output();
        }
        return await new Command(programOrArgv, argsOrOptions, maybeOptions).output();
    }

    function execSync(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return programOrArgv.outputSync();
        }
        if (arguments.length === 1) return new Command(programOrArgv).outputSync();
        if (arguments.length === 2) {
            return new Command(programOrArgv, argsOrOptions).outputSync();
        }
        return new Command(programOrArgv, argsOrOptions, maybeOptions).outputSync();
    }

    async function run(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return await programOrArgv.status();
        }
        if (arguments.length === 1) return await new Command(programOrArgv).status();
        if (arguments.length === 2) {
            return await new Command(programOrArgv, argsOrOptions).status();
        }
        return await new Command(programOrArgv, argsOrOptions, maybeOptions).status();
    }

    function runSync(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return programOrArgv.statusSync();
        }
        if (arguments.length === 1) return new Command(programOrArgv).statusSync();
        if (arguments.length === 2) {
            return new Command(programOrArgv, argsOrOptions).statusSync();
        }
        return new Command(programOrArgv, argsOrOptions, maybeOptions).statusSync();
    }

    function spawnFn(programOrArgv, argsOrOptions, maybeOptions) {
        if (arguments.length === 1 && programOrArgv instanceof Command) {
            return programOrArgv.spawn();
        }
        if (arguments.length === 1) return new Command(programOrArgv).spawn();
        if (arguments.length === 2) {
            return new Command(programOrArgv, argsOrOptions).spawn();
        }
        return new Command(programOrArgv, argsOrOptions, maybeOptions).spawn();
    }

    async function whichFn(name, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("which takes one or two arguments");
        }
        return unwrapExec(await opWhichAsync(name, options));
    }

    function whichSync(name, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("whichSync takes one or two arguments");
        }
        return unwrapExec(opWhichSync(name, options));
    }

    async function whichAllFn(name, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("whichAll takes one or two arguments");
        }
        return unwrapExec(await opWhichAllAsync(name, options));
    }

    function whichAllSync(name, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("whichAllSync takes one or two arguments");
        }
        return unwrapExec(opWhichAllSync(name, options));
    }

    async function lockFile(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("lockFile takes one or two arguments");
        }
        const res = await opLockAsync(path, options);
        if (!res.ok) throwExec(res);
        return new FileLock(lockToken, res.id, path, res.mode);
    }

    function lockFileSync(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("lockFileSync takes one or two arguments");
        }
        const res = opLockSync(path, options);
        if (!res.ok) throwExec(res);
        return new FileLock(lockToken, res.id, path, res.mode);
    }

    return freeze({
        ChildProcess,
        Command,
        CommandError,
        FileLock,
        NotFoundOnPathError,
        Output,
        PathFinder,
        addLogger,
        clearLoggers,
        cmd,
        exec,
        execSync,
        lockFile,
        lockFileSync,
        pathFinder,
        run,
        runSync,
        spawn: spawnFn,
        which: whichFn,
        whichAll: whichAllFn,
        whichAllSync,
        whichSync,
    });
})
