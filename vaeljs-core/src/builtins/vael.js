// vaeljs built-in modules.
//
// This script is evaluated once per realm before the entry module. It installs
// the global `Vael` object and the module objects that object caches.
// `Vael.engine` is `"vael"` for this V8 runtime, and
// `Vael.getBuiltInModulesList` returns every canonical `vael:` module specifier
// as one frozen array. `Vael.getBuiltInModule` takes the canonical specifier; a
// bare registry name is not a module key and returns `undefined`. `assert`
// calls one private host op that observes a genuine native Promise; `console`
// is plain ECMAScript and `chars` and `strings` are native Rust bindings
// installed before this shim; `crypto` calls thirty-two private host ops:
// four use the operating system CSPRNG and AES-256-GCM in
// `vaeljs-core/src/crypto.rs`; eight implement bounded AES-256-CBC/AES-256-GCM
// ciphers in `vaeljs-core/src/crypto_cipher.rs`; seven implement SHA-2 hashing and HMAC in
// `vaeljs-core/src/crypto_hash.rs`; seven implement PBKDF2 and bcrypt in
// `vaeljs-core/src/crypto_hash/password.rs`; and six implement GF(256) Shamir
// secret sharing in `vaeljs-core/src/crypto_shamir.rs`. The hash-family and
// Shamir ops include bounded asynchronous calls on the runtime's Tokio blocking
// pool; the hash and cipher families also have explicit incremental-object
// disposal; `age` calls four private host ops that use the
// `neotales-age` crate
// for binary age v1 X25519 encryption; `stdio` calls four
// private host ops that copy or encode bytes, copy a byte range out of a private
// snapshot, and write to the current process's standard output and standard
// error. `ssh` calls three private host ops that dial one host, verify an
// explicit host-key pin or known-hosts document, authenticate, and run one
// command at a time. None of them is a permission or sandbox
// and none is gated on a secure context or TLS. `stdio` adds no filesystem,
// process, environment, or stdin capability. `ssh` adds outbound TCP and SSH
// command execution; an explicit Unix agent opt-in reads SSH_AUTH_SOCK after
// host verification, but other modes read no environment and no agent. It adds
// no filesystem, stdin, or permission capability and never forwards an agent.
// `uuid` calls two private host ops that generate RFC 9562 version 4 and
// version 7 UUIDs from the operating system CSPRNG and return canonical
// lowercase text; it has no insecure fallback and promises no monotonic order.
// `fs` calls fourteen private host ops in `vaeljs-core/src/fs.rs` backed by the
// engine-neutral `vaeljs-fs-core` crate for synchronous filesystem operations.
// The `console` module is the realm's installed console
// object, not a separate console or an isolation boundary. The `test` module
// runs in the realm without a private host op: it registers sync and async
// tests, reports failures, pending tests, timeouts, and unhandled rejections,
// and uses the runtime's existing print, timer, and event-loop primitives. It
// is not Node's `node:test` and is not an isolation boundary.
//
// The `cron` module parses UTC five-field cron expressions and runs a callback
// on the schedule through six private host ops. It keeps one callback at a time,
// reports overlap, missed ticks, and callback errors through an optional
// observer, and keeps the event loop alive until the job is disposed. It has no
// local-time, DST, time-zone, or fake-timer surface, and it is not Deno's
// `Deno.cron`, Bun's `Bun.cron`, or `node-cron`.
//
// The `os` module exposes only the build target's `platform` and `arch`. They
// are Rust's `std::env::consts` values substituted before this script runs; the
// module calls no host op, reads no environment or operating-system file, and
// installs no global.
//
// The `chars` and `strings` modules are native Rust bindings shared with VaelJS
// Lite through the engine-neutral `vaeljs-builtins` crate. The runtime creates the
// function objects and registers the native V8 modules before this shim runs, so
// the registry, `slices`, and each module namespace all see one set of function
// objects. Character classification and simple case folding use the crate's
// Unicode 16.0 tables, not V8's, so both engines classify and fold the same input
// the same way.
(() => {
    "use strict";

    const MAX_CODE_POINT = 0x10ffff;
    const SURROGATE_MIN = 0xd800;
    const SURROGATE_MAX = 0xdfff;
    const ASCII_MAX = 0x7f;
    const LATIN1_MAX = 0xff;

    // Capture the built-ins used below so later replacements in the same realm
    // do not change classification. This is not isolation from code that
    // replaced them before this script ran, and it is not a sandbox.
    const isInteger = Number.isInteger;
    const fromCharCode = String.fromCharCode;
    const toWellFormed =
        typeof String.prototype.toWellFormed === "function"
            ? String.prototype.toWellFormed
            : null;
    // Capture `exec`, not `test`. `RegExp.prototype.test` looks up `this.exec`
    // dynamically, so replacing `RegExp.prototype.exec` would change what the
    // predicates answer or make them throw. The captured intrinsic does not
    // consult that mutable property.
    const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);
    const lowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);
    // Look up modules through a bound intrinsic, not `modules.get`. A dynamic
    // property lookup would consult the mutable `Map.prototype.get`, letting
    // later code replace lookups or receive the private registry map itself.
    // The same reasoning applies to the one-time iteration that builds the
    // public module list.
    const mapGet = Function.prototype.call.bind(Map.prototype.get);
    const mapForEachEntry = Function.prototype.call.bind(Map.prototype.forEach);
    const makeError = Error;
    const makeTypeError = TypeError;
    const makeRangeError = RangeError;
    const defineProperty = Object.defineProperty;
    const freeze = Object.freeze;
    const objectKeys = Object.keys;
    const arrayIsArray = Array.isArray;
    const arrayIncludes = Function.prototype.call.bind(Array.prototype.includes);
    // The stdio module builds and reads typed arrays without consulting mutable
    // globals or prototype methods. These captures are not isolation from code
    // that replaced them before this script ran.
    const makeUint8Array = Uint8Array;
    // `set` copies without a species lookup. `subarray` is deliberately not
    // captured or used: it calls `Symbol.species`, which could pass the private
    // snapshot buffer to user JavaScript and re-enter the module. Range copies
    // go through the native slice op instead.
    const typedArraySet = Function.prototype.call.bind(Uint8Array.prototype.set);
    // `%TypedArray%.prototype.indexOf` validates attached and in-bounds state
    // (including for resizable-ArrayBuffer views) in O(1) when passed `Infinity`
    // as `fromIndex`, without consulting prototype mutations or reading elements.
    const typedArrayIndexOf = Function.prototype.call.bind(
        Object.getPrototypeOf(Uint8Array.prototype).indexOf,
    );

    // The shared `chars` module is a native V8 object installed by Rust before
    // this shim runs. The registry entry below spreads that object in; the
    // object and every function are native V8 values. `slices` reads the same
    // object from the internal entries global at evaluation time.

    // `strings` is a native V8 object installed by Rust before this shim runs.
    // The registry entry below refers to that object; every export is a native
    // V8 function and the same function object is the matching registry
    // property and module export. There is no ECMAScript implementation or
    // module facade. `slices` reads the same native `chars` object from the
    // internal entries global at evaluation time.

    // Intrinsics shared by the JavaScript modules below. Capturing them at shim
    // evaluation means a later replacement does not change those modules. This
    // is not isolation from code that replaced an intrinsic before this script
    // ran, and it is not a sandbox.
    const makeRegExp = RegExp;
    const stringSlice = Function.prototype.call.bind(String.prototype.slice);
    const setAdd = Function.prototype.call.bind(Set.prototype.add);
    const setHas = Function.prototype.call.bind(Set.prototype.has);
    const mapSet = Function.prototype.call.bind(Map.prototype.set);

    // The host ops for `crypto` are captured now, before user code runs. Later
    // replacement of `Deno.core.ops` therefore does not change what the module
    // calls. This is not isolation from code that replaced them before this
    // script ran, and it is not a sandbox.
    const cryptoOps = Deno.core.ops;
    const opGenerateEnvelopeKey = cryptoOps.op_vael_crypto_generate_envelope_key;
    const opRandomBytes = cryptoOps.op_vael_crypto_random_bytes;
    const opEncrypt = cryptoOps.op_vael_crypto_encrypt;
    const opDecrypt = cryptoOps.op_vael_crypto_decrypt;
    const opCipherCreate = cryptoOps.op_vael_crypto_cipher_create;
    const opCipherUpdate = cryptoOps.op_vael_crypto_cipher_update;
    const opCipherFinal = cryptoOps.op_vael_crypto_cipher_final;
    const opCipherSetAad = cryptoOps.op_vael_crypto_cipher_set_aad;
    const opCipherSetTag = cryptoOps.op_vael_crypto_cipher_set_tag;
    const opCipherGetTag = cryptoOps.op_vael_crypto_cipher_get_tag;
    const opCipherSetPadding = cryptoOps.op_vael_crypto_cipher_set_padding;
    const opCipherRelease = cryptoOps.op_vael_crypto_cipher_release;
    const opHashCreate = cryptoOps.op_vael_crypto_hash_create;
    const opHmacCreate = cryptoOps.op_vael_crypto_hmac_create;
    const opHashRelease = cryptoOps.op_vael_crypto_hash_release;
    const opHashUpdate = cryptoOps.op_vael_crypto_hash_update;
    const opHashDigest = cryptoOps.op_vael_crypto_hash_digest;
    const opAsyncSubmit = cryptoOps.op_vael_crypto_async_submit;
    const opAsyncJoin = cryptoOps.op_vael_crypto_async_join;
    const opPbkdf2 = cryptoOps.op_vael_crypto_pbkdf2;
    const opPbkdf2Async = cryptoOps.op_vael_crypto_pbkdf2_async_submit;
    const opPasswordJoin = cryptoOps.op_vael_crypto_password_async_join;
    const opBcryptHash = cryptoOps.op_vael_crypto_bcrypt_hash;
    const opBcryptHashAsync = cryptoOps.op_vael_crypto_bcrypt_hash_async_submit;
    const opBcryptVerify = cryptoOps.op_vael_crypto_bcrypt_verify;
    const opBcryptVerifyAsync = cryptoOps.op_vael_crypto_bcrypt_verify_async_submit;
    const opShamirSplit = cryptoOps.op_vael_crypto_shamir_split;
    const opShamirSplitAsync = cryptoOps.op_vael_crypto_shamir_split_async_submit;
    const opShamirCombine = cryptoOps.op_vael_crypto_shamir_combine;
    const opShamirCombineAsync = cryptoOps.op_vael_crypto_shamir_combine_async_submit;
    const opShamirJoin = cryptoOps.op_vael_crypto_shamir_async_join;
    const opShamirTake = cryptoOps.op_vael_crypto_shamir_take_result;

    // The age host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the age module calls. The ops
    // are synchronous and return a result object; the wrapper below throws.
    const ageOps = Deno.core.ops;
    const opAgeGenerateIdentity = ageOps.op_vael_age_generate_identity;
    const opAgeIdentityToRecipient = ageOps.op_vael_age_identity_to_recipient;
    const opAgeEncrypt = ageOps.op_vael_age_encrypt;
    const opAgeDecrypt = ageOps.op_vael_age_decrypt;

    // The stdio host ops are captured for the same reason: replacing
    // `Deno.core.ops` later does not change what the classes call. They
    // validate views, copy byte ranges natively, and write to the current
    // process's standard streams; they do not grant filesystem, process,
    // environment, or stdin access.
    const stdioOps = Deno.core.ops;
    const opStdioCopy = stdioOps.op_vael_stdio_copy;
    const opStdioViewLength = stdioOps.op_vael_stdio_view_length;
    const opStdioSlice = stdioOps.op_vael_stdio_slice;
    const opStdioWrite = stdioOps.op_vael_stdio_write;

    // The ssh host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the module calls. They dial
    // one host, verify an explicit host-key pin or known-hosts document,
    // authenticate, and run one command at a time. They add no filesystem,
    // environment, agent, or stdin access.
    const sshOps = Deno.core.ops;
    const opSshConnect = sshOps.op_vael_ssh_connect;
    const opSshExec = sshOps.op_vael_ssh_exec;
    const opSshClose = sshOps.op_vael_ssh_close;

    // The uuid host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the module calls. They draw
    // from the operating system CSPRNG and return canonical text; neither takes
    // an argument, and neither falls back to a weaker source.
    const uuidOps = Deno.core.ops;
    const opUuidV4 = uuidOps.op_vael_uuid_v4;
    const opUuidV7 = uuidOps.op_vael_uuid_v7;

    // The fs host ops are captured here for the same reason: replacing
    // `Deno.core.ops` later does not change what the module calls. They perform
    // synchronous filesystem operations via `vaeljs-fs-core`.
    const fsOps = Deno.core.ops;
    const opFsReadFile = fsOps.op_vael_fs_read_file;
    const opFsReadTextFile = fsOps.op_vael_fs_read_text_file;
    const opFsWriteFile = fsOps.op_vael_fs_write_file;
    const opFsWriteTextFile = fsOps.op_vael_fs_write_text_file;
    const opFsWriteFileAtomic = fsOps.op_vael_fs_write_file_atomic;
    const opFsExists = fsOps.op_vael_fs_exists;
    const opFsStat = fsOps.op_vael_fs_stat;
    const opFsLstat = fsOps.op_vael_fs_lstat;
    const opFsReadDir = fsOps.op_vael_fs_read_dir;
    const opFsMkdir = fsOps.op_vael_fs_mkdir;
    const opFsRemove = fsOps.op_vael_fs_remove;
    const opFsRename = fsOps.op_vael_fs_rename;
    const opFsCopyFile = fsOps.op_vael_fs_copy_file;
    const opFsRealpath = fsOps.op_vael_fs_realpath;

    /**
     * Throw the exception described by a failed host result.
     *
     * Argument errors are `TypeError` or `RangeError` and carry no code. Other
     * failures are `Error`; `code` is defined as an own property instead of
     * assigned, so an inherited `Object.prototype.code` accessor cannot swallow
     * it. The host result object has a null prototype, so its properties cannot
     * be intercepted either.
     *
     * @param {{ok: boolean, name?: string, code?: string, message?: string}} response
     * @returns {never}
     */
    function throwCrypto(response) {
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        if (response.name === "RangeError") {
            throw new makeRangeError(response.message);
        }
        const error = new makeError(response.message);
        defineProperty(error, "code", {
            __proto__: null,
            value: response.code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        throw error;
    }

    /**
     * Turn the host result object into a value or the documented exception.
     *
     * @param {{ok: boolean, value?: Uint8Array, name?: string, code?: string, message?: string}} response
     * @returns {Uint8Array}
     */
    function unwrapCrypto(response) {
        if (response.ok) return response.value;
        throwCrypto(response);
    }

    /**
     * Turn a host result object that carries a resource id into the id.
     *
     * @param {{ok: boolean, id?: number, name?: string, code?: string, message?: string}} response
     * @returns {number}
     */
    function unwrapId(response) {
        if (response.ok) return response.id;
        throwCrypto(response);
    }

    /** Throw unless a host result object reports success. */
    function unwrapUnit(response) {
        if (!response.ok) throwCrypto(response);
    }

    /**
     * The ordinary `Error` this module reports for hash lifecycle and worker
     * failures, with a stable own `code`.
     */
    function hashError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /**
     * Generate a 32-byte AES-256 key from the operating system CSPRNG.
     *
     * This is the envelope key generator. `generateKey` is not exported in this
     * slice: that name is reserved for a future Node-compatible surface.
     *
     * Keep the key secret, use a distinct key per purpose, and retire it well
     * before about 2^32 messages under it (the AES-GCM nonce-collision risk).
     * The module stores no keys and does not promise zeroization.
     *
     * @returns {Uint8Array}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `VAEL_CRYPTO_RANDOM` when the CSPRNG fails.
     */
    function generateEnvelopeKey() {
        if (arguments.length !== 0) {
            throw new makeTypeError("generateEnvelopeKey takes no arguments");
        }
        return unwrapCrypto(opGenerateEnvelopeKey());
    }

    /**
     * Return `length` bytes from the operating system CSPRNG.
     *
     * For salts, IVs, or nonces used by another construction. This is the same
     * CSPRNG source as `generateEnvelopeKey`; there is no weaker fallback, and
     * a failure throws rather than returning predictable bytes. The call is
     * synchronous. It is not a statistical test of the output, and it does not
     * prove cryptographic security.
     *
     * There is no callback form in this slice and none is reserved:
     * asynchronous APIs will be `Promise`-based. Passing a second argument
     * throws instead of silently ignoring it.
     *
     * @param {number} length a non-negative integer, at most 8 MiB
     * @returns {Uint8Array}
     * @throws {TypeError} when `length` is not a number or the argument count
     * is not exactly one.
     * @throws {RangeError} when `length` is not an integer or is outside the
     * documented range.
     * @throws {Error} with `code` `VAEL_CRYPTO_RANDOM` when the OS CSPRNG fails.
     */
    function randomBytes(length) {
        if (arguments.length !== 1) {
            throw new makeTypeError("randomBytes takes exactly one argument");
        }
        return unwrapCrypto(opRandomBytes(length));
    }

    /**
     * Encrypt `plaintext` into a fresh versioned envelope.
     *
     * The nonce is drawn fresh for every call and the header, nonce, and
     * `associatedData` are authenticated. Synchronous; every argument and the
     * result are `Uint8Array`s.
     *
     * Limits: `plaintext` is limited to 8 MiB, `associatedData` to 8 MiB, and
     * the returned envelope to 8 MiB + 34 bytes. These are vaeljs limits, not
     * intrinsic AES-GCM limits.
     *
     * Security: a fresh random 96-bit nonce is used per call. Under one key the
     * nonce-collision probability reaches about 2^-32 after roughly 2^32
     * encryptions, so retire the key well before that. Never reuse a key with a
     * repeated nonce. The module does not track key use or promise zeroization.
     *
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} plaintext
     * @param {Uint8Array} [associatedData] authenticated but not encrypted
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument is not a `Uint8Array` or the
     * argument count is not two or three.
     * @throws {Error} with `code` `VAEL_CRYPTO_KEY`, `VAEL_CRYPTO_RANDOM`,
     * `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`.
     */
    function encrypt(key, plaintext, associatedData) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("encrypt takes two or three arguments");
        }
        return unwrapCrypto(opEncrypt(key, plaintext, associatedData));
    }

    /**
     * Verify and decrypt one envelope.
     *
     * The whole header and nonce, the associated data, and the ciphertext are
     * authenticated. No plaintext is returned when the tag does not verify.
     *
     * Limits: the envelope is limited to 8 MiB + 34 bytes and `associatedData`
     * to 8 MiB. These are vaeljs limits, not intrinsic AES-GCM limits.
     *
     * Security: a failed tag check returns no plaintext. AES-GCM needs unique
     * nonces under one key, so retire a key well before about 2^32 messages
     * under it. This module does not track or enforce key use, and it does not
     * promise zeroization.
     *
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} envelope
     * @param {Uint8Array} [associatedData] must match the value used to encrypt
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument is not a `Uint8Array` or the
     * argument count is not two or three.
     * @throws {Error} with `code` `VAEL_CRYPTO_AUTH`, `VAEL_CRYPTO_FORMAT`,
     * `VAEL_CRYPTO_KEY`, `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`.
     */
    function decrypt(key, envelope, associatedData) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("decrypt takes two or three arguments");
        }
        return unwrapCrypto(opDecrypt(key, envelope, associatedData));
    }

    // Stateful AES-256-CBC and AES-256-GCM. See
    // `vaeljs-core/src/crypto_cipher.rs`. `update` buffers a bounded amount of
    // input and returns an empty Uint8Array; `final` performs the operation and
    // returns the output. GCM never releases plaintext before its tag check.

    const cipherToken = Symbol("vaeljs.crypto.Cipher");

    /**
     * Normalize an algorithm name without exposing an argument-coercion hook.
     *
     * @param {string} algorithm
     * @returns {string} the lowercased name, which the host op still validates
     */
    function cipherAlgorithmName(algorithm) {
        if (typeof algorithm !== "string") {
            throw new makeTypeError("algorithm must be a string");
        }
        return lowerCase(algorithm);
    }

    /**
     * A bounded stateful cipher created by `createEncryptor` or
     * `createDecryptor`.
     *
     * `update(data[, encoding])` copies bytes or a UTF-8 string into the
     * object's bounded buffer and returns an empty `Uint8Array`. `final()`
     * performs the operation over the whole buffered message and marks the
     * object finalized. For GCM encryption, `getAuthTag()` is available only
     * after `final()`. For GCM decryption, `setAuthTag(tag)` is required before
     * `final()`, and `final()` returns no plaintext when the tag does not
     * verify. A closed object's methods throw `VAEL_CRYPTO_CLOSED`; an operation
     * after `final()` throws `VAEL_CRYPTO_STATE`.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after a
     * successful `final()` or a thrown `final()`. Finalization releases the
     * buffered message, but the object keeps its native slot until it is closed
     * or the runtime is dropped. The per-runtime open-object and aggregate
     * buffer caps make explicit disposal mandatory for long-lived code.
     *
     * This is not Node's `Cipher` class and does not promise Node's chunk
     * timing, output encodings, or callback form.
     */
    class Cipher {
        #id;
        #encrypt;
        #gcm;
        #closed = false;
        #finalized = false;

        constructor(token, id, encrypt, gcm) {
            if (token !== cipherToken) {
                throw new makeTypeError(
                    "Cipher objects cannot be constructed directly; use createEncryptor() or createDecryptor()",
                );
            }
            this.#id = id;
            this.#encrypt = encrypt;
            this.#gcm = gcm;
        }

        #assertOpen() {
            if (this.#closed) {
                throw hashError("VAEL_CRYPTO_CLOSED", "the cipher object is closed");
            }
        }

        #assertBuffering() {
            this.#assertOpen();
            if (this.#finalized) {
                throw hashError("VAEL_CRYPTO_STATE", "the cipher has already been finalized");
            }
        }

        /**
         * Buffer bytes or a UTF-8 string for the cipher operation.
         *
         * @param {Uint8Array | string} data
         * @param {string} [encoding] only `utf8`/`utf-8`, and only for a string
         * @returns {Uint8Array} always empty in this buffered slice
         * @throws {TypeError} for a wrong argument count or type, an encoding
         * supplied with binary input, or an unsupported input encoding.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`.
         */
        update(data, encoding) {
            if (arguments.length < 1 || arguments.length > 2) {
                throw new makeTypeError("update takes one or two arguments");
            }
            this.#assertBuffering();
            let code;
            if (typeof data === "string") {
                code = inputEncodingCode(encoding);
            } else {
                if (encoding !== undefined) {
                    throw new makeTypeError("encoding must not be supplied for binary input");
                }
                code = undefined;
            }
            unwrapUnit(opCipherUpdate(this.#id, data, code));
            return new makeUint8Array(0);
        }

        /**
         * Finish the operation and return its output.
         *
         * For CBC this verifies and removes PKCS#7 padding, or checks the
         * no-padding block length. For GCM decryption this checks the
         * authentication tag before returning any plaintext.
         *
         * @returns {Uint8Array} ciphertext or plaintext
         * @throws {TypeError} when any argument is supplied.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * `VAEL_CRYPTO_AUTH`, `VAEL_CRYPTO_PADDING`, `VAEL_CRYPTO_TAG`, or
         * `VAEL_CRYPTO_LIMIT`.
         */
        final() {
            if (arguments.length !== 0) {
                throw new makeTypeError("final takes no arguments");
            }
            this.#assertBuffering();
            this.#finalized = true;
            return unwrapCrypto(opCipherFinal(this.#id));
        }

        /**
         * Set GCM associated data, at most once and before `update`/`final`.
         *
         * @param {Uint8Array} aad authenticated but not encrypted
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count, a non-`Uint8Array`,
         * or a detached view.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * `VAEL_CRYPTO_UNSUPPORTED`, or `VAEL_CRYPTO_LIMIT`.
         */
        setAAD(aad) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAAD takes exactly one argument");
            }
            this.#assertBuffering();
            if (!this.#gcm) {
                throw hashError(
                    "VAEL_CRYPTO_UNSUPPORTED",
                    "associated data is only supported for aes-256-gcm",
                );
            }
            unwrapUnit(opCipherSetAad(this.#id, aad));
            return this;
        }

        /**
         * Set the expected 16-byte GCM authentication tag before `final`.
         *
         * @param {Uint8Array} tag exactly 16 bytes
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count or a non-`Uint8Array`.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * `VAEL_CRYPTO_UNSUPPORTED`, or `VAEL_CRYPTO_TAG`.
         */
        setAuthTag(tag) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAuthTag takes exactly one argument");
            }
            this.#assertBuffering();
            if (!this.#gcm || this.#encrypt) {
                throw hashError(
                    "VAEL_CRYPTO_UNSUPPORTED",
                    "setAuthTag is only supported on aes-256-gcm decryptors",
                );
            }
            unwrapUnit(opCipherSetTag(this.#id, tag));
            return this;
        }

        /**
         * Return the 16-byte GCM tag produced by `final`.
         *
         * @returns {Uint8Array}
         * @throws {TypeError} when any argument is supplied.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * or `VAEL_CRYPTO_UNSUPPORTED`.
         */
        getAuthTag() {
            if (arguments.length !== 0) {
                throw new makeTypeError("getAuthTag takes no arguments");
            }
            this.#assertOpen();
            if (!this.#gcm || !this.#encrypt) {
                throw hashError(
                    "VAEL_CRYPTO_UNSUPPORTED",
                    "getAuthTag is only supported on aes-256-gcm encryptors",
                );
            }
            if (!this.#finalized) {
                throw hashError("VAEL_CRYPTO_STATE", "final must be called before getAuthTag");
            }
            return unwrapCrypto(opCipherGetTag(this.#id));
        }

        /**
         * Enable or disable CBC PKCS#7 padding before `final`.
         *
         * With padding disabled, the plaintext or ciphertext must already be a
         * multiple of 16 bytes.
         *
         * @param {boolean} enabled
         * @returns {Cipher} this object, for chaining
         * @throws {TypeError} for a wrong argument count or a non-boolean.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`, `VAEL_CRYPTO_STATE`,
         * or `VAEL_CRYPTO_UNSUPPORTED`.
         */
        setAutoPadding(enabled) {
            if (arguments.length !== 1) {
                throw new makeTypeError("setAutoPadding takes exactly one argument");
            }
            this.#assertBuffering();
            if (this.#gcm) {
                throw hashError(
                    "VAEL_CRYPTO_UNSUPPORTED",
                    "padding is not supported for aes-256-gcm",
                );
            }
            if (typeof enabled !== "boolean") {
                throw new makeTypeError("setAutoPadding expects a boolean");
            }
            unwrapUnit(opCipherSetPadding(this.#id, enabled));
            return this;
        }

        /**
         * Release this object's native state.
         *
         * Idempotent. A later method throws `VAEL_CRYPTO_CLOSED`. Closing is the
         * deterministic way to stay below the per-runtime open-object and
         * buffer caps.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opCipherRelease(this.#id);
        }

        /**
         * Alias for `close()`, for explicit-disposal call sites.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        dispose() {
            if (arguments.length !== 0) {
                throw new makeTypeError("dispose takes no arguments");
            }
            this.close();
        }
    }

    /**
     * Create a bounded stateful encryptor for AES-256-CBC or AES-256-GCM.
     *
     * The caller supplies the IV/nonce and must keep it unique: never reuse a
     * GCM nonce under one key, and use an unpredictable CBC IV per key. This
     * module does not generate, count, or track IVs.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after
     * `final()` or a thrown `final()`. Finalization frees the message buffer
     * but the object keeps its native slot until it is closed or the runtime is
     * dropped, so explicit disposal is required to stay below the per-runtime
     * open-object cap and to release the object deterministically.
     *
     * `options` must be `undefined` or a plain or null-prototype non-Proxy
     * record whose own properties are enumerable data properties with string
     * keys. Validation is own-only: own symbols, accessors, and non-enumerable
     * properties are rejected, and a custom prototype is rejected. Inherited
     * properties on the intrinsic `Object.prototype` or a null prototype are
     * ignored, including a polluted `Object.prototype.padding`. Arrays, class
     * instances, functions, and Proxies throw `TypeError` before a resource is
     * created. CBC accepts only `padding`; GCM accepts none.
     *
     * @param {"aes-256-cbc" | "aes-256-gcm"} algorithm case-insensitive
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} iv 16 bytes for CBC, 12 bytes for GCM
     * @param {{padding?: boolean}} [options] CBC only; no other key is accepted
     * @returns {Cipher}
     * @throws {TypeError} for a wrong argument count or an unsupported option.
     * @throws {Error} with `code` `VAEL_CRYPTO_ALGORITHM`, `VAEL_CRYPTO_KEY`,
     * `VAEL_CRYPTO_IV`, `VAEL_CRYPTO_DETACHED`, or
     * `VAEL_CRYPTO_TOO_MANY_OBJECTS`.
     */
    function createEncryptor(algorithm, key, iv, options) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("createEncryptor takes three or four arguments");
        }
        const name = cipherAlgorithmName(algorithm);
        const id = unwrapId(opCipherCreate(0, name, key, iv, options));
        return new Cipher(cipherToken, id, true, name === "aes-256-gcm");
    }

    /**
     * Create a bounded stateful decryptor for AES-256-CBC or AES-256-GCM.
     *
     * GCM decryptors require `setAuthTag(tag)` before `final()`. No plaintext
     * is returned from `update`, so an unauthenticated message cannot leak
     * plaintext before the tag check. CBC decryption is unauthenticated; if it
     * is needed for interoperability, verify an independent encrypt-then-MAC
     * tag over the IV and ciphertext before calling this object, and do not
     * expose padding validity as an oracle.
     *
     * Always call `close()`/`dispose()` in a `finally` block, even after
     * `final()` or a thrown `final()`. Finalization frees the message buffer
     * but the object keeps its native slot until it is closed or the runtime is
     * dropped, so explicit disposal is required to stay below the per-runtime
     * open-object cap and to release the object deterministically.
     *
     * `options` must be `undefined` or a plain or null-prototype non-Proxy
     * record whose own properties are enumerable data properties with string
     * keys. Validation is own-only: own symbols, accessors, and non-enumerable
     * properties are rejected, and a custom prototype is rejected. Inherited
     * properties on the intrinsic `Object.prototype` or a null prototype are
     * ignored, including a polluted `Object.prototype.padding`. Arrays, class
     * instances, functions, and Proxies throw `TypeError` before a resource is
     * created. CBC accepts only `padding`; GCM accepts none.
     *
     * @param {"aes-256-cbc" | "aes-256-gcm"} algorithm case-insensitive
     * @param {Uint8Array} key exactly 32 bytes
     * @param {Uint8Array} iv 16 bytes for CBC, 12 bytes for GCM
     * @param {{padding?: boolean}} [options] CBC only; no other key is accepted
     * @returns {Cipher}
     * @throws {TypeError} for a wrong argument count or an unsupported option.
     * @throws {Error} with `code` `VAEL_CRYPTO_ALGORITHM`, `VAEL_CRYPTO_KEY`,
     * `VAEL_CRYPTO_IV`, `VAEL_CRYPTO_DETACHED`, or
     * `VAEL_CRYPTO_TOO_MANY_OBJECTS`.
     */
    function createDecryptor(algorithm, key, iv, options) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("createDecryptor takes three or four arguments");
        }
        const name = cipherAlgorithmName(algorithm);
        const id = unwrapId(opCipherCreate(1, name, key, iv, options));
        return new Cipher(cipherToken, id, false, name === "aes-256-gcm");
    }

    // Hash and HMAC. The synchronous factories return an incremental object
    // with `update` and `digest`. The asynchronous helpers are one-shot and
    // return a Promise resolved by a bounded worker pool; they are not
    // Promise-wrapped blocking calls. See `vaeljs-core/src/crypto_hash.rs`.

    const hashToken = Symbol("vaeljs.crypto.Hash");

    /**
     * Return the numeric code for a digest output encoding.
     *
     * The accepted names are `hex`, `base64`, `base64url`, `latin1`, and the
     * Node alias `binary`. `undefined` means raw bytes. Any other value throws
     * `TypeError`, and the hash state is not consumed.
     *
     * @param {string | undefined} encoding
     * @returns {number} 0 raw, 1 hex, 2 base64, 3 base64url, 4 latin1
     */
    function outputEncodingCode(encoding) {
        if (encoding === undefined) return 0;
        if (typeof encoding !== "string") {
            throw new makeTypeError("encoding must be a string");
        }
        const name = lowerCase(encoding);
        if (name === "hex") return 1;
        if (name === "base64") return 2;
        if (name === "base64url") return 3;
        if (name === "latin1" || name === "binary") return 4;
        throw new makeTypeError("unsupported digest encoding");
    }

    /**
     * Return the numeric code for a string input encoding.
     *
     * Only UTF-8 is accepted, as `utf8` or `utf-8`, case-insensitively, and as
     * the default when the argument is `undefined`. Binary input may not name
     * an encoding at all.
     *
     * @param {string | undefined} encoding
     * @returns {number} always 0
     */
    function inputEncodingCode(encoding) {
        if (encoding === undefined) return 0;
        if (typeof encoding !== "string") {
            throw new makeTypeError("encoding must be a string");
        }
        const name = lowerCase(encoding);
        if (name === "utf8" || name === "utf-8") return 0;
        throw new makeTypeError("unsupported input encoding");
    }

    /** Map byte values 0-255 to one JavaScript code unit each. */
    function latin1String(bytes) {
        let text = "";
        for (let index = 0; index < bytes.length; index += 1) {
            text += fromCharCode(bytes[index]);
        }
        return text;
    }

    /**
     * A synchronous incremental SHA-2 hash or HMAC.
     *
     * Created by `createHash` or `createHmac`, not by user code. `update`
     * copies the current view's bytes or encodes the string as UTF-8, so later
     * mutation of the input does not change the result. `digest` consumes the
     * state; a second `digest` or a later `update` throws
     * `VAEL_CRYPTO_DIGESTED`. `close()`/`dispose()` release the state without a
     * digest; later calls throw `VAEL_CRYPTO_CLOSED`. Open objects are capped per
     * runtime, so close objects you do not finish.
     */
    class Hash {
        #id;
        #digested = false;
        #closed = false;

        constructor(token, id) {
            if (token !== hashToken) {
                throw new makeTypeError(
                    "Hash objects cannot be constructed directly; use createHash() or createHmac()",
                );
            }
            this.#id = id;
        }

        /**
         * Add bytes or a UTF-8 string to the hash.
         *
         * @param {Uint8Array | string} data
         * @param {string} [encoding] only `utf8`/`utf-8`, and only for a string
         * @returns {Hash} this object, for chaining
         * @throws {TypeError} for a wrong argument count or type, an encoding
         * supplied with binary input, or an unsupported input encoding.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`,
         * `VAEL_CRYPTO_DIGESTED`, `VAEL_CRYPTO_LIMIT`, or
         * `VAEL_CRYPTO_DETACHED`.
         */
        update(data, encoding) {
            if (arguments.length < 1 || arguments.length > 2) {
                throw new makeTypeError("update takes one or two arguments");
            }
            if (this.#closed) {
                throw hashError("VAEL_CRYPTO_CLOSED", "the hash object is closed");
            }
            if (this.#digested) {
                throw hashError("VAEL_CRYPTO_DIGESTED", "digest has already been called");
            }
            const isText = typeof data === "string";
            if (!isText && encoding !== undefined) {
                throw new makeTypeError("encoding must not be supplied for binary input");
            }
            const code = isText ? inputEncodingCode(encoding) : undefined;
            unwrapUnit(opHashUpdate(this.#id, data, code));
            return this;
        }

        /**
         * Finish the hash and return the digest.
         *
         * `digest()` returns a `Uint8Array`. A string encoding returns a
         * string: lowercase hex, padded standard Base64, unpadded URL-safe
         * Base64, or one code unit per byte for `latin1`/`binary`. An
         * unsupported encoding is rejected before the state is consumed.
         *
         * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
         * @returns {Uint8Array | string}
         * @throws {TypeError} for a second argument when the count is over one
         * or an unsupported encoding.
         * @throws {Error} with `code` `VAEL_CRYPTO_CLOSED`,
         * `VAEL_CRYPTO_DIGESTED`, or `VAEL_CRYPTO_LIMIT`.
         */
        digest(encoding) {
            if (arguments.length > 1) {
                throw new makeTypeError("digest takes at most one argument");
            }
            if (this.#closed) {
                throw hashError("VAEL_CRYPTO_CLOSED", "the hash object is closed");
            }
            if (this.#digested) {
                throw hashError("VAEL_CRYPTO_DIGESTED", "digest has already been called");
            }
            const code = outputEncodingCode(encoding);
            this.#digested = true;
            const response = opHashDigest(this.#id, code);
            if (!response.ok) throwCrypto(response);
            return response.value;
        }

        /**
         * Release this object's native state without producing a digest.
         *
         * Idempotent. A later `update` or `digest` throws `VAEL_CRYPTO_CLOSED`.
         * Closing is the deterministic way to stay below the per-runtime open
         * object limit; an object that is never closed or digested keeps its
         * slot until the runtime is dropped.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opHashRelease(this.#id);
        }

        /**
         * Alias for `close()`, for explicit-disposal call sites.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        dispose() {
            if (arguments.length !== 0) {
                throw new makeTypeError("dispose takes no arguments");
            }
            this.close();
        }
    }

    /**
     * Create an incremental SHA-2 hash.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @returns {Hash}
     * @throws {TypeError} when the argument count is not one or the value is
     * not a string.
     * @throws {Error} with `code` `VAEL_CRYPTO_ALGORITHM` for another name, or
     * `VAEL_CRYPTO_TOO_MANY_OBJECTS` when the per-runtime open-object cap is
     * reached. Close objects you will not digest with `close()`/`dispose()`;
     * an abandoned object counts against the cap until the runtime is dropped.
     */
    function createHash(algorithm) {
        if (arguments.length !== 1) {
            throw new makeTypeError("createHash takes exactly one argument");
        }
        return new Hash(hashToken, unwrapId(opHashCreate(algorithm)));
    }

    /**
     * Create an incremental HMAC.
     *
     * The key is a `Uint8Array` of exact bytes or a UTF-8 string. HMAC accepts
     * a key of any length, including zero. The key is copied now, so mutating
     * the caller's view afterwards does not change the MAC.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} key
     * @returns {Hash}
     * @throws {TypeError} when the argument count is not two or a value has
     * the wrong type.
     * @throws {Error} with `code` `VAEL_CRYPTO_ALGORITHM`,
     * `VAEL_CRYPTO_TOO_MANY_OBJECTS` when the per-runtime open-object cap is
     * reached, `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`. Close objects you
     * will not digest with `close()`/`dispose()`; an abandoned object counts
     * against the cap until the runtime is dropped.
     */
    function createHmac(algorithm, key) {
        if (arguments.length !== 2) {
            throw new makeTypeError("createHmac takes exactly two arguments");
        }
        return new Hash(hashToken, unwrapId(opHmacCreate(algorithm, key)));
    }

    /**
     * Await one asynchronous digest job and decode its bytes.
     *
     * Calling the join op removes the job from the resource table immediately.
     * An unawaited result is retained by this promise, not by the table. The
     * join op resolves to an empty `Uint8Array` only when the worker failed; a
     * real digest is never empty, so that sentinel becomes
     * `VAEL_CRYPTO_INTERNAL`.
     */
    async function joinDigest(id, code) {
        const bytes = await opAsyncJoin(id);
        if (bytes.length === 0) {
            throw hashError("VAEL_CRYPTO_INTERNAL", "the hash worker failed");
        }
        return code === 0 ? bytes : latin1String(bytes);
    }

    /**
     * Hash one input on the runtime's bounded worker pool.
     *
     * Genuinely asynchronous: validation, budget admission, and a single bounded
     * copy of `data` into a module-owned buffer run before the returned Promise
     * exists, then all hashing runs on a blocking worker and the Promise
     * resolves on the isolate thread.
     * It does not wrap a blocking call in `Promise.resolve`. The input is copied
     * before the promise is returned, so mutating it afterwards does not change
     * the digest. The join starts immediately: ignoring the returned Promise
     * does not cancel the work, and dropping only the `run_file_async` entry
     * future does not either. Only runtime teardown aborts a queued job; a
     * running job cannot be stopped. At most 64 jobs, even empty ones, and 64 MiB of
     * copied data and HMAC key bytes may be pending; a submission over either
     * bound rejects with `VAEL_CRYPTO_BUSY`.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} data a string is encoded as UTF-8
     * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
     * @returns {Promise<Uint8Array | string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function hashAsync(algorithm, data, encoding) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("hashAsync takes two or three arguments");
        }
        const code = outputEncodingCode(encoding);
        const id = unwrapId(opAsyncSubmit(algorithm, 0, undefined, data, code));
        return joinDigest(id, code);
    }

    /**
     * Compute an HMAC on the runtime's bounded worker pool.
     *
     * The asynchronous form of `createHmac(algorithm, key).update(data).digest
     * (encoding)`. The key and data are copied before the promise is returned.
     * The join starts immediately; ignoring the returned Promise does not
     * cancel the work. Only runtime teardown aborts a queued job; a running job
     * cannot be stopped.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} key a string is encoded as UTF-8
     * @param {Uint8Array | string} data a string is encoded as UTF-8
     * @param {"hex" | "base64" | "base64url" | "latin1" | "binary"} [encoding]
     * @returns {Promise<Uint8Array | string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function hmacAsync(algorithm, key, data, encoding) {
        if (arguments.length < 3 || arguments.length > 4) {
            throw new makeTypeError("hmacAsync takes three or four arguments");
        }
        const code = outputEncodingCode(encoding);
        const id = unwrapId(opAsyncSubmit(algorithm, 1, key, data, code));
        return joinDigest(id, code);
    }

    // PBKDF2 and bcrypt. These share the hash worker's bounded executor and
    // pending-job budget. The sync forms block the isolate; the async forms
    // return a Promise resolved by the same blocking pool. See
    // `vaeljs-core/src/crypto_hash/password.rs`.

    /**
     * Await one asynchronous PBKDF2 or bcrypt job and decode its bytes.
     *
     * Calling the join op removes the job from the resource table immediately.
     * An empty result is the worker-failure sentinel. `kind` is 0 for raw
     * PBKDF2 bytes, 1 for an ASCII bcrypt hash, and 2 for a verification
     * boolean encoded as one byte.
     */
    async function joinPassword(id, kind) {
        const bytes = await opPasswordJoin(id);
        // An empty buffer means the join/resource was missing or the worker
        // panicked. A tagged buffer carries either success or a stable worker
        // failure category, so a failed CSPRNG draw is not flattened into an
        // internal error.
        if (bytes.length === 0) {
            throw hashError(
                "VAEL_CRYPTO_INTERNAL",
                "the password hashing worker failed",
            );
        }
        const tag = bytes[0];
        if (tag === 1) {
            throw hashError(
                "VAEL_CRYPTO_RANDOM",
                "operating system random number generator failed",
            );
        }
        if (tag === 2) {
            throw hashError(
                "VAEL_CRYPTO_LIMIT",
                "input exceeds the password hashing size limit",
            );
        }
        if (tag !== 0) {
            throw hashError(
                "VAEL_CRYPTO_INTERNAL",
                "the password hashing worker failed",
            );
        }
        if (kind === 1) {
            // The bcrypt hash is ASCII. Build the string from the payload
            // without slicing the host buffer through a mutable prototype.
            let text = "";
            for (let index = 1; index < bytes.length; index += 1) {
                text += fromCharCode(bytes[index]);
            }
            return text;
        }
        if (kind === 2) return bytes[1] !== 0;
        // PBKDF2 output. Copy the payload without `subarray`/`slice`, whose
        // species lookup could run user JavaScript.
        const output = new makeUint8Array(bytes.length - 1);
        for (let index = 1; index < bytes.length; index += 1) {
            output[index - 1] = bytes[index];
        }
        return output;
    }

    /**
     * Derive key material with PBKDF2.
     *
     * Synchronous: it blocks the isolate for the whole derivation. Use
     * `pbkdf2Async` to keep the event loop moving. `password` and `salt` are
     * exact `Uint8Array` bytes or strings encoded as UTF-8. This is not Node's
     * `crypto.pbkdf2Sync` and does not accept a digest name such as `sha1`.
     *
     * Limits: `iterations` is 1 through 1,000,000, `length` is 1 through 1024
     * bytes, and one password or salt is at most 1 MiB. These are vaeljs limits,
     * not PBKDF2 limits. A string is measured by its UTF-8 length before it is
     * copied.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} password
     * @param {Uint8Array | string} salt
     * @param {number} iterations integer 1 through 1,000,000
     * @param {number} length integer 1 through 1024
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly five.
     * @throws {RangeError} when `iterations` or `length` is outside its range.
     * @throws {Error} with `code` `VAEL_CRYPTO_ALGORITHM`,
     * `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`.
     */
    function pbkdf2(algorithm, password, salt, iterations, length) {
        if (arguments.length !== 5) {
            throw new makeTypeError("pbkdf2 takes exactly five arguments");
        }
        return unwrapCrypto(opPbkdf2(algorithm, password, salt, iterations, length));
    }

    /**
     * Derive key material with PBKDF2 on the runtime's bounded worker pool.
     *
     * Genuinely asynchronous: validation, budget admission, and one bounded
     * copy into a module-owned zeroizing buffer run on the isolate thread, then
     * the derivation runs on a blocking worker. It is not a
     * resolved-`Promise` wrapper around blocking work. The
     * inputs are copied before the promise is returned. The join starts
     * immediately; ignoring the returned Promise does not cancel the work, and
     * dropping only the `run_file_async` entry future does not either. Only
     * runtime teardown aborts a queued job; a running job cannot be stopped and
     * stays charged until it ends.
     *
     * Limits are the same as `pbkdf2`. PBKDF2 and bcrypt share the hash
     * worker's bounds: at most 64 jobs and 64 MiB of copied input may be
     * pending. A submission over either bound rejects with
     * `VAEL_CRYPTO_BUSY`.
     *
     * @param {"sha256" | "sha384" | "sha512"} algorithm case-insensitive
     * @param {Uint8Array | string} password
     * @param {Uint8Array | string} salt
     * @param {number} iterations integer 1 through 1,000,000
     * @param {number} length integer 1 through 1024
     * @returns {Promise<Uint8Array>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function pbkdf2Async(algorithm, password, salt, iterations, length) {
        if (arguments.length !== 5) {
            throw new makeTypeError("pbkdf2Async takes exactly five arguments");
        }
        const id = unwrapId(
            opPbkdf2Async(algorithm, password, salt, iterations, length),
        );
        return joinPassword(id, 0);
    }

    /**
     * Hash a password with bcrypt.
     *
     * Returns a 60-character modular-crypt string in the `$2b$` format. The
     * salt is 16 bytes from the operating system CSPRNG. The password is exact
     * bytes or a UTF-8 string; at most 72 bytes are accepted, and exactly 72
     * bytes are hashed as 72 bytes with no added NUL. A longer password throws
     * `VAEL_CRYPTO_LIMIT`; it is never silently truncated.
     *
     * `cost` is the logarithmic work factor, an integer from 4 through 16,
     * defaulting to 10. The upper bound is a vaeljs policy that keeps a single
     * call bounded; it is deliberately lower than bcrypt's format maximum.
     *
     * This is synchronous and blocks the isolate. Use `bcryptHashAsync` to
     * keep the event loop moving.
     *
     * @param {Uint8Array | string} password
     * @param {number} [cost=10] integer 4 through 16
     * @returns {string}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not one or two.
     * @throws {RangeError} when `cost` is not an integer in its range.
     * @throws {Error} with `code` `VAEL_CRYPTO_LIMIT`, `VAEL_CRYPTO_RANDOM`, or
     * `VAEL_CRYPTO_DETACHED`.
     */
    function bcryptHash(password, cost) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("bcryptHash takes one or two arguments");
        }
        return unwrapCrypto(opBcryptHash(password, cost));
    }

    /**
     * Hash a password with bcrypt on the runtime's bounded worker pool.
     *
     * The asynchronous form of `bcryptHash`, with the same password and cost
     * bounds. Argument validation, budget admission, and the copy run before
     * the promise is returned; the password is hashed on a blocking worker. The
     * join starts immediately; ignoring the returned Promise does not cancel
     * the work. Only runtime teardown aborts a queued job; a running job cannot
     * be stopped.
     *
     * @param {Uint8Array | string} password
     * @param {number} [cost=10] integer 4 through 16
     * @returns {Promise<string>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function bcryptHashAsync(password, cost) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("bcryptHashAsync takes one or two arguments");
        }
        const id = unwrapId(opBcryptHashAsync(password, cost));
        return joinPassword(id, 1);
    }

    /**
     * Verify a password against a bcrypt modular-crypt string.
     *
     * Accepts `$2a$`, `$2b$`, and `$2y$` hashes. A malformed string, an
     * unsupported version (`$2x$` included), or an encoded cost outside 4
     * through 16 returns `false` without computing a hash, so a malicious hash
     * cannot force an unbounded work factor. A password longer than 72 bytes
     * also returns `false`; it is not an error and is never truncated to a
     * partial match.
     *
     * The comparison inside the bcrypt primitive is constant-time. This is
     * synchronous and blocks the isolate for the work factor in `hash`; use
     * `bcryptVerifyAsync` to keep the event loop moving.
     *
     * @param {Uint8Array | string} password
     * @param {string} hash
     * @returns {boolean}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `VAEL_CRYPTO_LIMIT` or
     * `VAEL_CRYPTO_DETACHED` for an input that is too large or detached.
     */
    function bcryptVerify(password, hash) {
        if (arguments.length !== 2) {
            throw new makeTypeError("bcryptVerify takes exactly two arguments");
        }
        return unwrapCrypto(opBcryptVerify(password, hash));
    }

    /**
     * Verify a bcrypt hash on the runtime's bounded worker pool.
     *
     * A well-formed hash runs on a blocking worker. An oversized password or a
     * malformed or over-cost hash resolves to `false` without starting a
     * worker, because there is no expensive work to do. An invalid argument
     * type still rejects. When a worker is started, the join starts
     * immediately; ignoring the returned Promise does not cancel the work. Only
     * runtime teardown aborts a queued job; a running job cannot be stopped.
     *
     * @param {Uint8Array | string} password
     * @param {string} hash
     * @returns {Promise<boolean>}
     * @throws rejects with `TypeError` or an `Error` carrying a stable `code`.
     */
    async function bcryptVerifyAsync(password, hash) {
        if (arguments.length !== 2) {
            throw new makeTypeError("bcryptVerifyAsync takes exactly two arguments");
        }
        const response = opBcryptVerifyAsync(password, hash);
        if (!response.ok) throwCrypto(response);
        if (typeof response.value === "boolean") return response.value;
        return joinPassword(response.id, 2);
    }

    // GF(256) Shamir secret sharing. See `vaeljs-core/src/crypto_shamir.rs`.
    // Shares are bytewise compatible with the Neotales PLAB and rust-libraries
    // encodings, but the JavaScript objects are this module's own shape.

    /**
     * The ordinary `Error` this module reports for Shamir worker failures.
     *
     * @param {string} code
     * @param {string} message
     * @returns {Error}
     */
    function shamirError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /**
     * Freeze a host-produced share record.
     *
     * The object's `index` property cannot be replaced afterwards, but its
     * `value` bytes remain an ordinary mutable `Uint8Array`.
     *
     * @param {{index: number, value: Uint8Array}} share
     * @returns {{index: number, value: Uint8Array}}
     */
    function freezeShare(share) {
        return freeze({ index: share.index, value: share.value });
    }

    /**
     * Copy host-produced share records into frozen objects.
     *
     * @param {Array<{index: number, value: Uint8Array}>} shares
     * @returns {Array<{index: number, value: Uint8Array}>}
     */
    function freezeShares(shares) {
        const out = [];
        for (let position = 0; position < shares.length; position += 1) {
            // Define an own data property. Assigning `out[position]` would walk
            // Array.prototype and could invoke a poisoned numeric setter.
            defineProperty(out, position, {
                __proto__: null,
                value: freezeShare(shares[position]),
                writable: true,
                enumerable: true,
                configurable: true,
            });
        }
        return out;
    }

    /**
     * Split a secret into shares using Shamir secret sharing over GF(256).
     *
     * The field uses reduction polynomial `0x11b`, generator `0x03`, and
     * one-based share indexes, so the emitted share values are bytewise
     * compatible with the Neotales PLAB .NET and rust-libraries Shamir
     * encodings. Every non-constant polynomial coefficient comes from the
     * operating-system CSPRNG; there is no fallback. A CSPRNG failure throws
     * `VAEL_CRYPTO_RANDOM` rather than returning predictable shares.
     *
     * Every share has the same length as `secret`, so the secret length is
     * public. The share values are copies; mutating one does not change the
     * others. The returned records are frozen but their `Uint8Array` values are
     * mutable. This function is synchronous; `shamirSplitAsync` is the
     * nonblocking form.
     *
     * Shares are not authenticated. Tampered shares, shares from different
     * splits, and too few shares reconstruct a different byte string, often
     * without any error. Use a separate authenticated envelope or out-of-band
     * check if wrong reconstruction must be detected.
     *
     * A `Uint8Array` backed by a `SharedArrayBuffer` is rejected with
     * `VAEL_CRYPTO_SHAMIR_SHARED`. Copying shared memory is not one coherent
     * snapshot: another agent can change it mid-copy. If shared memory is the
     * source, copy a synchronized snapshot into an ordinary non-shared buffer
     * first.
     *
     * Limits: `secret` is at most 8 MiB, `threshold` and `shareCount` are
     * integers from 2 through 255 with `shareCount >= threshold`, and
     * `secret.length * shareCount` is at most 64 MiB.
     *
     * @param {Uint8Array} secret
     * @param {number} threshold
     * @param {number} shareCount
     * @returns {Array<{index: number, value: Uint8Array}>}
     * @throws {TypeError} for a wrong argument count, a non-`Uint8Array`
     * secret, or a non-number threshold or share count.
     * @throws {RangeError} for an empty secret or a threshold or share count
     * outside its documented range.
     * @throws {Error} with `code` `VAEL_CRYPTO_LIMIT`, `VAEL_CRYPTO_RANDOM`,
     * `VAEL_CRYPTO_DETACHED`, or `VAEL_CRYPTO_SHAMIR_SHARED`.
     */
    function shamirSplit(secret, threshold, shareCount) {
        if (arguments.length !== 3) {
            throw new makeTypeError("shamirSplit takes exactly three arguments");
        }
        const response = opShamirSplit(secret, threshold, shareCount);
        return freezeShares(unwrapCrypto(response));
    }

    /**
     * Split a secret on the runtime's bounded blocking pool.
     *
     * Validation, budget admission, and one bounded copy of `secret` run before
     * the returned Promise exists; the random coefficients and all polynomial
     * arithmetic run on a blocking worker. Copying the finished shares into
     * JavaScript happens when the Promise resolves. This is not a resolved
     * Promise around a blocking call. Mutating `secret` after the call does not
     * change the result. The join starts immediately, so ignoring the Promise
     * does not cancel the work; only runtime teardown aborts a queued job.
     *
     * The same limits, share encoding, randomness source, authenticity, and
     * shared-buffer limitations as `shamirSplit` apply. Pending work shares the
     * `hashAsync` budget of at most 64 jobs and 64 MiB of the worker's exact
     * owned peak: the secret copy, one packed output, and the temporary noise
     * buffer. A submission over either bound rejects with `VAEL_CRYPTO_BUSY`.
     * The finished result is held in a zeroizing resource until it is
     * transferred to a zeroizing V8 backing store; if the runtime is dropped
     * first, the resource drop clears it.
     *
     * @param {Uint8Array} secret
     * @param {number} threshold
     * @param {number} shareCount
     * @returns {Promise<Array<{index: number, value: Uint8Array}>>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function shamirSplitAsync(secret, threshold, shareCount) {
        if (arguments.length !== 3) {
            throw new makeTypeError("shamirSplitAsync takes exactly three arguments");
        }
        const id = unwrapId(opShamirSplitAsync(secret, threshold, shareCount));
        const status = await opShamirJoin(id);
        const response = opShamirTake(id, 1);
        if (status.length !== 0) {
            // The worker reported a stable category; `take` also releases the
            // resource and returns the matching failure object.
            if (response.ok) {
                throw shamirError("VAEL_CRYPTO_INTERNAL", "the Shamir worker failed");
            }
            throwCrypto(response);
        }
        return freezeShares(unwrapCrypto(response));
    }

    /**
     * Recover a secret from Shamir shares over GF(256).
     *
     * The shares are interpolated at `x = 0`; interpolation coefficients are
     * computed once from the public indexes, not once per secret byte. When
     * `threshold` is supplied it must be an integer from 2 through 255 and at
     * least that many shares must be present; omitting it accepts any set of at
     * least two shares.
     *
     * This function does not authenticate shares. A genuine reconstruction is
     * only possible when the shares are untampered and from the same split. A
     * tampered set, a mixed set, or fewer than the original threshold produces
     * a byte string that is not the secret, and this function cannot tell.
     * Verify the result with a separate authenticated envelope or out-of-band
     * digest if that matters. The returned bytes are a new `Uint8Array`.
     *
     * A share `value` backed by a `SharedArrayBuffer` is rejected with
     * `VAEL_CRYPTO_SHAMIR_SHARED`. Copying shared memory is not one coherent
     * snapshot: another agent can change it mid-copy. If shared memory is the
     * source, copy a synchronized snapshot into an ordinary non-shared buffer
     * first.
     *
     * Limits: at least two and at most 255 shares are accepted, every share
     * value must have the same non-zero length of at most 8 MiB, and the total
     * share bytes are at most 64 MiB. The per-share limit is checked before
     * any value is copied and does not depend on the sync or async path.
     *
     * @param {Array<{index: number, value: Uint8Array}>} shares
     * @param {number} [threshold]
     * @returns {Uint8Array}
     * @throws {TypeError} for a wrong argument count, a non-array `shares`, a
     * non-object share, an accessor property, a missing or non-number index, or
     * a non-`Uint8Array` value.
     * @throws {RangeError} for a malformed count, a zero or out-of-range index,
     * an empty or unequal share value, or a supplied threshold that is invalid
     * or larger than the share count.
     * @throws {Error} with `code` `VAEL_CRYPTO_SHAMIR_DUPLICATE`,
     * `VAEL_CRYPTO_SHAMIR_LENGTH`, `VAEL_CRYPTO_SHAMIR_SHARED`,
     * `VAEL_CRYPTO_LIMIT`, or `VAEL_CRYPTO_DETACHED`.
     */
    function shamirCombine(shares, threshold) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("shamirCombine takes one or two arguments");
        }
        const response = opShamirCombine(shares, threshold);
        return unwrapCrypto(response);
    }

    /**
     * Recover a secret from Shamir shares on the runtime's bounded blocking
     * pool.
     *
     * Validation, budget admission, and one bounded copy of every share value
     * run before the returned Promise exists; interpolation runs on a blocking
     * worker. Copying the recovered bytes into JavaScript happens when the
     * Promise resolves. Mutating the share objects or their values after the
     * call does not change the result. The same authenticity and threshold
     * limitations as `shamirCombine` apply.
     *
     * Pending work shares the `hashAsync` budget of at most 64 jobs and 64 MiB
     * of the worker's exact owned peak: the copied shares, the recovered
     * secret, and the public interpolation basis. A submission over either
     * bound rejects with `VAEL_CRYPTO_BUSY`. The finished result is held in a
     * zeroizing resource until it is transferred to a zeroizing V8 backing
     * store; if the runtime is dropped first, the resource drop clears it.
     *
     * @param {Array<{index: number, value: Uint8Array}>} shares
     * @param {number} [threshold]
     * @returns {Promise<Uint8Array>}
     * @throws rejects with `TypeError`, `RangeError`, or an `Error` carrying a
     * stable `code`.
     */
    async function shamirCombineAsync(shares, threshold) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("shamirCombineAsync takes one or two arguments");
        }
        const id = unwrapId(opShamirCombineAsync(shares, threshold));
        const status = await opShamirJoin(id);
        const response = opShamirTake(id, 2);
        if (status.length !== 0) {
            if (response.ok) {
                throw shamirError("VAEL_CRYPTO_INTERNAL", "the Shamir worker failed");
            }
            throwCrypto(response);
        }
        return unwrapCrypto(response);
    }

    // A frozen module object, like `chars`: the properties cannot be replaced,
    // but the function objects themselves are shared with the imports.
    const crypto = Object.freeze({
        generateEnvelopeKey,
        randomBytes,
        encrypt,
        decrypt,
        createEncryptor,
        createDecryptor,
        createHash,
        createHmac,
        hashAsync,
        hmacAsync,
        pbkdf2,
        pbkdf2Async,
        bcryptHash,
        bcryptHashAsync,
        bcryptVerify,
        bcryptVerifyAsync,
        shamirSplit,
        shamirSplitAsync,
        shamirCombine,
        shamirCombineAsync,
    });

    /**
     * Turn an age host result object into a value or the documented exception.
     *
     * A host-side argument error is `TypeError`; every other failure is `Error`
     * with an own `code` defined through the captured `Object.defineProperty`,
     * so an inherited `Object.prototype.code` accessor cannot swallow it. The
     * host result object has a null prototype, so its properties cannot be
     * intercepted either.
     *
     * @param {{ok: boolean, value?: string, identity?: string, recipient?: string, name?: string, code?: string, message?: string}} response
     * @returns {string | Uint8Array | undefined}
     */
    function unwrapAge(response) {
        if (response.ok) return response.value;
        let error;
        if (response.name === "TypeError") {
            error = new makeTypeError(response.message);
        } else {
            error = new makeError(response.message);
            defineProperty(error, "code", {
                __proto__: null,
                value: response.code,
                writable: true,
                enumerable: true,
                configurable: true,
            });
        }
        throw error;
    }

    /**
     * Generate a new X25519 identity and its recipient.
     *
     * The secret comes from the operating system CSPRNG through the
     * `neotales-age` backend; an entropy failure throws rather than returning
     * predictable bytes. The identity is `AGE-SECRET-KEY-1...` and the recipient
     * is `age1...`, both in standard Bech32 text.
     *
     * Keep the identity secret. This module stores no keys, does not derive or
     * rotate them for the caller, and does not promise zeroization.
     *
     * @returns {{identity: string, recipient: string}}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `VAEL_AGE_RANDOM` when the OS CSPRNG fails.
     */
    function generateIdentity() {
        if (arguments.length !== 0) {
            throw new makeTypeError("generateIdentity takes no arguments");
        }
        const response = opAgeGenerateIdentity();
        if (!response.ok) unwrapAge(response);
        return { identity: response.identity, recipient: response.recipient };
    }

    /**
     * Derive the X25519 recipient for an identity.
     *
     * Only standard age X25519 identities are accepted. A valid age key of
     * another kind that parses within the 512-byte bound (for example a
     * post-quantum identity) is rejected with `VAEL_AGE_UNSUPPORTED`, not
     * re-interpreted. Text that is not an age identity at all, including SSH or
     * passphrase text, is `VAEL_AGE_IDENTITY`. Leading and trailing whitespace
     * is trimmed; the text is bounded to 512 bytes before parsing.
     *
     * @param {string} identity an `AGE-SECRET-KEY-1...` identity
     * @returns {string} the matching `age1...` recipient
     * @throws {TypeError} when the argument is not a string or the argument
     * count is not exactly one.
     * @throws {Error} with `code` `VAEL_AGE_IDENTITY`, `VAEL_AGE_UNSUPPORTED`, or
     * `VAEL_AGE_INTERNAL`.
     */
    function identityToRecipient(identity) {
        if (arguments.length !== 1) {
            throw new makeTypeError("identityToRecipient takes exactly one argument");
        }
        if (typeof identity !== "string") {
            throw new makeTypeError("identity must be a string");
        }
        return unwrapAge(opAgeIdentityToRecipient(identity));
    }

    /**
     * Encrypt bytes to one X25519 recipient as a binary age v1 file.
     *
     * The result is a standard binary age file, decryptable by the `age` or
     * `rage` command-line tools. Inputs and output are `Uint8Array`s; the
     * recipient is Bech32 text and is never interpreted as bytes. A recipient
     * whose X25519 public key is a low-order point cannot produce a usable
     * shared secret and is rejected with `VAEL_AGE_RECIPIENT`.
     *
     * Limits: `plaintext` is limited to 8 MiB and the ciphertext to 8 MiB plus
     * 1 MiB for the header and per-chunk tags. A key or plaintext over the bound
     * throws before copying or allocating. This is a vaeljs limit, not an age
     * format limit.
     *
     * Security: the age crate draws the file key and payload nonce from the
     * operating system CSPRNG. A failure throws `VAEL_AGE_RANDOM`; there is no
     * weaker fallback. The module does not store keys or promise zeroization.
     *
     * @param {string} recipient an `age1...` X25519 recipient
     * @param {Uint8Array} plaintext
     * @returns {Uint8Array} a binary age v1 file
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `VAEL_AGE_RECIPIENT`, `VAEL_AGE_UNSUPPORTED`,
     * `VAEL_AGE_LIMIT`, `VAEL_AGE_RANDOM`, `VAEL_AGE_DETACHED`, or
     * `VAEL_AGE_INTERNAL`.
     */
    function ageEncrypt(recipient, plaintext) {
        if (arguments.length !== 2) {
            throw new makeTypeError("encrypt takes exactly two arguments");
        }
        if (typeof recipient !== "string") {
            throw new makeTypeError("recipient must be a string");
        }
        return unwrapAge(opAgeEncrypt(recipient, plaintext));
    }

    /**
     * Verify and decrypt one binary age v1 file with one X25519 identity.
     *
     * The header MAC and every payload chunk tag are verified by the backend.
     * No plaintext is returned when authentication fails, and a truncated or
     * tampered file throws instead of returning partial bytes. Only the binary
     * format is accepted; ASCII armor, passphrases, SSH keys, and other
     * identity kinds are not implemented in this slice.
     *
     * Limits: the ciphertext is limited to 8 MiB + 1 MiB and the recovered
     * plaintext to 8 MiB. The payload limit is enforced while chunks are
     * authenticated, so an oversized result fails rather than buffering.
     *
     * @param {string} identity an `AGE-SECRET-KEY-1...` X25519 identity
     * @param {Uint8Array} ciphertext a binary age v1 file
     * @returns {Uint8Array}
     * @throws {TypeError} when an argument has the wrong type or the argument
     * count is not exactly two.
     * @throws {Error} with `code` `VAEL_AGE_AUTH`, `VAEL_AGE_FORMAT`,
     * `VAEL_AGE_IDENTITY`, `VAEL_AGE_IDENTITY_MISMATCH`, `VAEL_AGE_UNSUPPORTED`,
     * `VAEL_AGE_LIMIT`, `VAEL_AGE_DETACHED`, or `VAEL_AGE_INTERNAL`.
     */
    function ageDecrypt(identity, ciphertext) {
        if (arguments.length !== 2) {
            throw new makeTypeError("decrypt takes exactly two arguments");
        }
        if (typeof identity !== "string") {
            throw new makeTypeError("identity must be a string");
        }
        return unwrapAge(opAgeDecrypt(identity, ciphertext));
    }

    // A frozen module object, like `chars` and `crypto`: the properties cannot
    // be replaced, but the function objects themselves are shared with imports.
    const age = Object.freeze({
        generateIdentity,
        identityToRecipient,
        encrypt: ageEncrypt,
        decrypt: ageDecrypt,
    });

    /**
     * Turn a uuid host result object into a value or the documented exception.
     *
     * The host result object has a null prototype, so its properties cannot be
     * intercepted. The `code` is defined as an own property instead of assigned,
     * so an inherited `Object.prototype.code` accessor cannot swallow it.
     *
     * @param {{ok: boolean, value?: string, name?: string, code?: string, message?: string}} response
     * @returns {string}
     */
    function unwrapUuid(response) {
        if (response.ok) return response.value;
        const error = new makeError(response.message);
        defineProperty(error, "code", {
            __proto__: null,
            value: response.code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        // The host sends only a fixed diagnostic: an OS error number or one of
        // getrandom's fixed categories. It never sends entropy, UUID bytes, a
        // clock value, or the OS error text. `cause` is non-enumerable like a
        // native Error cause.
        if (typeof response.cause === "string") {
            defineProperty(error, "cause", {
                __proto__: null,
                value: response.cause,
                writable: true,
                enumerable: false,
                configurable: true,
            });
        }
        throw error;
    }

    /**
     * Generate a new RFC 9562 version 4 UUID.
     *
     * The 122 payload bits come from the operating system CSPRNG; the four
     * version bits are `0100` and the two variant bits are `10`. The result is
     * canonical lowercase text, 36 characters long. A version 4 UUID carries no
     * timestamp or sequence, so it has no order and must not be used as a sort
     * key.
     *
     * There is no insecure fallback: an entropy failure throws instead of
     * returning predictable or time-derived bytes.
     *
     * @returns {string}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `VAEL_UUID_RANDOM` when the OS CSPRNG fails.
     * Its optional `cause` is a fixed diagnostic such as
     * `getrandom: OS error 5`; it never contains entropy or UUID bytes.
     */
    function uuidV4() {
        if (arguments.length !== 0) {
            throw new makeTypeError("v4 takes no arguments");
        }
        return unwrapUuid(opUuidV4());
    }

    /**
     * Generate a new RFC 9562 version 7 UUID.
     *
     * The first 48 bits are the current Unix time in milliseconds, big-endian.
     * The version is `0111`, the variant is `10`, and the remaining 74 payload
     * bits come from the operating system CSPRNG. The result is canonical
     * lowercase text, 36 characters long.
     *
     * Ordering is only what the timestamp prefix provides: values from
     * different milliseconds sort by that timestamp when the system clock moves
     * forward, while values from the same millisecond are randomly ordered.
     * A clock step backwards can produce a value that sorts before an earlier
     * one, and values from different processes are not ordered. This function
     * does not promise monotonicity, uniqueness, or a total order; use an
     * explicit sequence or ordering field when the application needs one.
     *
     * A time before the Unix epoch or at or beyond 2^48 milliseconds is refused
     * rather than wrapped or clamped. There is no insecure fallback: an entropy
     * failure throws instead of returning predictable bytes.
     *
     * @returns {string}
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `VAEL_UUID_CLOCK` when the clock is outside the
     * 48-bit timestamp range, or `VAEL_UUID_RANDOM` when the OS CSPRNG fails.
     * A random failure's optional `cause` is a fixed diagnostic such as
     * `getrandom: OS error 5`; it never contains entropy or UUID bytes.
     */
    function uuidV7() {
        if (arguments.length !== 0) {
            throw new makeTypeError("v7 takes no arguments");
        }
        return unwrapUuid(opUuidV7());
    }

    // A frozen module object, like `chars`, `crypto`, and `age`: the properties
    // cannot be replaced, but the function objects themselves are shared with
    // imports. There is no default export.
    const uuid = Object.freeze({
        v4: uuidV4,
        v7: uuidV7,
    });

    // The `uuid/v4` and `uuid/v7` subpath modules are thin views over the two
    // functions above. They expose the function as both the named export and the
    // default export, matching the conventional `uuid/v4` import shape, while
    // the aggregate `uuid` module has no default. The registry objects and the
    // ES module namespaces therefore list the same properties, and every path
    // shares the one function object.
    const uuidV4Module = Object.freeze({
        default: uuidV4,
        v4: uuidV4,
    });
    const uuidV7Module = Object.freeze({
        default: uuidV7,
        v7: uuidV7,
    });

    /**
     * Turn an fs host result object into its value or the documented exception.
     *
     * The host result object has a null prototype so its properties cannot be
     * intercepted. `code` is defined as an own property via `Object.defineProperty`
     * so an inherited `Error.prototype.code` / `Object.prototype.code` setter
     * cannot observe or swallow it.
     */
    function throwFs(response) {
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        if (response.name === "RangeError") {
            throw new makeRangeError(response.message);
        }
        const error = new makeError(response.message);
        defineProperty(error, "code", {
            __proto__: null,
            value: response.code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        throw error;
    }

    function unwrapFs(response) {
        if (response.ok) return response.value;
        throwFs(response);
    }

    function unwrapFsUnit(response) {
        if (!response.ok) throwFs(response);
    }

    function fsReadFile(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("readFile takes one or two arguments");
        }
        return unwrapFs(opFsReadFile(path, options));
    }

    function fsReadTextFile(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("readTextFile takes one or two arguments");
        }
        return unwrapFs(opFsReadTextFile(path, options));
    }

    function fsWriteFile(path, data, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("writeFile takes two or three arguments");
        }
        if (typeof path !== "string") {
            throw new makeTypeError("path must be a string");
        }
        typedArrayIndexOf(data, 0, 1 / 0);
        unwrapFsUnit(opFsWriteFile(path, data, options));
    }

    function fsWriteTextFile(path, text, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("writeTextFile takes two or three arguments");
        }
        unwrapFsUnit(opFsWriteTextFile(path, text, options));
    }

    function fsWriteFileAtomic(path, data, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("writeFileAtomic takes two or three arguments");
        }
        if (typeof path !== "string") {
            throw new makeTypeError("path must be a string");
        }
        if (typeof data !== "string") {
            typedArrayIndexOf(data, 0, 1 / 0);
        }
        unwrapFsUnit(opFsWriteFileAtomic(path, data, options));
    }

    function fsExists(path) {
        if (arguments.length !== 1) {
            throw new makeTypeError("exists takes exactly one argument");
        }
        return unwrapFs(opFsExists(path));
    }

    function fsStat(path) {
        if (arguments.length !== 1) {
            throw new makeTypeError("stat takes exactly one argument");
        }
        return unwrapFs(opFsStat(path));
    }

    function fsLstat(path) {
        if (arguments.length !== 1) {
            throw new makeTypeError("lstat takes exactly one argument");
        }
        return unwrapFs(opFsLstat(path));
    }

    function fsReadDir(path) {
        if (arguments.length !== 1) {
            throw new makeTypeError("readDir takes exactly one argument");
        }
        return unwrapFs(opFsReadDir(path));
    }

    function fsMkdir(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("mkdir takes one or two arguments");
        }
        unwrapFsUnit(opFsMkdir(path, options));
    }

    function fsRemove(path, options) {
        if (arguments.length < 1 || arguments.length > 2) {
            throw new makeTypeError("remove takes one or two arguments");
        }
        unwrapFsUnit(opFsRemove(path, options));
    }

    function fsRename(from, to) {
        if (arguments.length !== 2) {
            throw new makeTypeError("rename takes exactly two arguments");
        }
        unwrapFsUnit(opFsRename(from, to));
    }

    function fsCopyFile(from, to, options) {
        if (arguments.length < 2 || arguments.length > 3) {
            throw new makeTypeError("copyFile takes two or three arguments");
        }
        unwrapFsUnit(opFsCopyFile(from, to, options));
    }

    function fsRealpath(path) {
        if (arguments.length !== 1) {
            throw new makeTypeError("realpath takes exactly one argument");
        }
        return unwrapFs(opFsRealpath(path));
    }

    const fs = freeze({
        readFile: fsReadFile,
        readTextFile: fsReadTextFile,
        writeFile: fsWriteFile,
        writeTextFile: fsWriteTextFile,
        writeFileAtomic: fsWriteFileAtomic,
        exists: fsExists,
        stat: fsStat,
        lstat: fsLstat,
        readDir: fsReadDir,
        mkdir: fsMkdir,
        remove: fsRemove,
        rename: fsRename,
        copyFile: fsCopyFile,
        realpath: fsRealpath,
    });

    // The ssh module is a synchronous command-execution slice. It is not a
    // shell, PTY, SCP, SFTP, port forwarder, or agent client, and it never
    // reads a file, environment variable, or default known-hosts path. Host
    // trust is mandatory and fail-closed: exactly one of `hostKey` or
    // `knownHosts` is required, and there is no unknown-host mode. The limits
    // below must match the constants in `vaeljs-core/src/ssh.rs`; the boundary
    // tests exercise both sides.
    const MAX_COMMAND_BYTES = 65536;
    const MAX_OUTPUT_BYTES = 8388608;
    const DEFAULT_CONNECT_TIMEOUT_MS = 15000;
    const DEFAULT_EXEC_TIMEOUT_MS = 30000;

    // The option names `connect` accepts. An unknown name is rejected rather
    // than ignored, so a misspelled or invented option cannot silently weaken
    // host verification or authentication. Keep this list in step with the
    // documentation in `vaeljs-core/src/ssh.rs`.
    const SSH_OPTION_KEYS = Object.freeze([
        "host",
        "port",
        "username",
        "password",
        "privateKey",
        "agent",
        "passphrase",
        "hostKey",
        "knownHosts",
        "connectTimeoutMs",
        "execTimeoutMs",
    ]);

    // The token that makes `SshConnection` constructible only by this shim.
    // It is module-private, so a caller cannot fabricate a connection handle.
    const sshConnectionToken = Symbol("vaeljs.ssh.Connection");

    /**
     * Create the ordinary `Error` this module reports, with a stable own
     * `code` and `name = "SshError"`. `code` is defined rather than assigned
     * so an inherited `Object.prototype.code` accessor cannot swallow it. The
     * message never contains a password, passphrase, key byte, command, or
     * output byte.
     */
    function sshError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "name", {
            __proto__: null,
            value: "SshError",
            writable: true,
            enumerable: false,
            configurable: true,
        });
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /** Throw the exception described by one host result object. */
    function throwSsh(response) {
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        if (response.name === "RangeError") {
            throw new makeRangeError(response.message);
        }
        throw sshError(response.code, response.message);
    }

    /**
     * One synchronous, authenticated SSH connection.
     *
     * Returned by `connect`, not constructible by user code. `exec` runs one
     * command and returns `{ stdout, status }`; stderr is discarded. Both
     * methods block the isolate. `close` is idempotent and closes the socket by
     * dropping the native connection; it does not send a graceful SSH
     * disconnect. A fatal `exec` failure also closes the connection.
     */
    class SshConnection {
        #id;
        #closed = false;

        constructor(token, id) {
            if (token !== sshConnectionToken) {
                throw new makeTypeError(
                    "SSH connections cannot be constructed directly; use connect()",
                );
            }
            this.#id = id;
        }

        /** Whether this connection can no longer run a command. */
        get closed() {
            return this.#closed;
        }

        /**
         * Run one command and return its captured stdout and exit status.
         *
         * `command` is a string encoded as UTF-8 or a `Uint8Array` of exact
         * bytes. The server's stderr is discarded. The command is bounded by
         * MAX_COMMAND_BYTES and captured stdout by MAX_OUTPUT_BYTES. The call
         * blocks until the exit status arrives and the session channel closes,
         * or until the connection's `execTimeoutMs` expires.
         *
         * @param {string | Uint8Array} command
         * @returns {{stdout: Uint8Array, status: number}}
         * @throws {TypeError} when the argument count is not one or the value
         * is neither a string nor a `Uint8Array`.
         * @throws {Error} with `code` `VAEL_SSH_CLOSED`, `VAEL_SSH_LIMIT`,
         * `VAEL_SSH_TIMEOUT`, `VAEL_SSH_IO`, `VAEL_SSH_SESSION`, or
         * `VAEL_SSH_INTERNAL`. A failure after the connection is engaged
         * (transport, session, timeout, or output over the limit) closes it.
         * Argument validation, including a command over MAX_COMMAND_BYTES,
         * throws `TypeError`, `RangeError`, or `VAEL_SSH_LIMIT` without
         * closing the connection; inspect `closed` after catching.
         */
        exec(command) {
            if (arguments.length !== 1) {
                throw new makeTypeError("exec takes exactly one argument");
            }
            if (this.#closed) {
                throw sshError("VAEL_SSH_CLOSED", "the SSH connection is closed");
            }
            const response = opSshExec(this.#id, command);
            if (!response.ok) {
                if (response.closed === true) this.#closed = true;
                throwSsh(response);
            }
            return { stdout: response.stdout, status: response.status };
        }

        /**
         * Close this connection. Idempotent.
         *
         * Dropping the native connection closes its TCP socket; no SSH
         * disconnect message is sent. Later `exec` calls throw
         * `VAEL_SSH_CLOSED`.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            opSshClose(this.#id);
        }
    }

    /**
     * Dial, verify, authenticate, and return a synchronous SSH connection.
     *
     * Exactly one of `hostKey` (an SSH-format Ed25519 public-key blob as a
     * `Uint8Array`) or `knownHosts` (the text of a known-hosts document) is
     * required. There is no insecure default, no first-use acceptance, and no
     * fallback to a file: a missing or mismatched host key refuses to connect.
     *
     * Exactly one of `password`, `privateKey`, or `agent: true` is required.
     * `privateKey` is an in-memory `Uint8Array` containing unencrypted PKCS#8
     * DER, a PKCS#8 PEM block, a binary `openssh-key-v1` container, or an
     * OpenSSH PEM block. A `passphrase` is accepted only with `privateKey`, is
     * used only for OpenSSH containers, and is wiped by the SSH backend after
     * parsing; supplying one with a PKCS#8 key is refused. Encrypted PKCS#8 is
     * not supported.
     *
     * `agent` is an explicit opt-in and is supported on Unix only. When true,
     * `SSH_AUTH_SOCK` is read and its identities are tried, at most 16, after
     * the server host key has been verified. At most two authentication
     * requests are made per identity, 32 total. The agent timeout is not a
     * hard wall clock: `UnixStream::connect` may block before any timeout is
     * installed, and the backend's 30-second read/write timeout is
     * inactivity-based, so a peer that trickles bytes can prolong enumeration
     * or a signing request beyond 30 seconds. It is independent of
     * `connectTimeoutMs`; this slice rejects Windows named pipes rather than
     * claiming a bounded pipe timeout. There is no agent forwarding and no
     * implicit agent use: other modes never read `SSH_AUTH_SOCK`. Apart from
     * that opt-in, this module never reads a file, environment variable, agent,
     * or terminal.
     *
     * `connectTimeoutMs` (1..120000, default 15000) bounds each TCP connect
     * attempt and is the socket inactivity timeout during the blocking
     * handshake and authentication; DNS resolution is outside it. The
     * handshake is one library call without an incremental deadline, so a
     * peer that stalls fails but one that trickles bytes can extend it.
     * `execTimeoutMs` (1..600000, default 30000) is checked between session
     * events and before their socket I/O for each later `exec`.
     *
     * The call blocks the isolate until the handshake and authentication
     * finish. It returns no `Promise` and takes no callback.
     *
     * @param {object} options
     * @param {string} options.host
     * @param {number} [options.port=22]
     * @param {string} options.username
     * @param {string} [options.password]
     * @param {Uint8Array} [options.privateKey]
     * @param {boolean} [options.agent]
     * @param {string} [options.passphrase]
     * @param {Uint8Array} [options.hostKey]
     * @param {string} [options.knownHosts]
     * @param {number} [options.connectTimeoutMs=15000]
     * @param {number} [options.execTimeoutMs=30000]
     * @returns {SshConnection}
     * @throws {TypeError} when the argument count is not one, `options` is not
     * a plain object, an unknown option name is supplied, or a value has the
     * wrong type.
     * @throws {RangeError} when a number is not an integer in its documented
     * range.
     * @throws {Error} with `code` `VAEL_SSH_HOST_KEY_REQUIRED`,
     * `VAEL_SSH_HOST_KEY`, `VAEL_SSH_KNOWN_HOSTS`, `VAEL_SSH_AUTH_METHOD`,
     * `VAEL_SSH_CREDENTIAL`, `VAEL_SSH_AGENT`, `VAEL_SSH_CONNECT`,
     * `VAEL_SSH_TIMEOUT`, `VAEL_SSH_HANDSHAKE`, `VAEL_SSH_AUTH`, `VAEL_SSH_LIMIT`,
     * or `VAEL_SSH_INTERNAL`.
     */
    function sshConnect(options) {
        if (arguments.length !== 1) {
            throw new makeTypeError("connect takes exactly one argument");
        }
        if (options === null || typeof options !== "object" || arrayIsArray(options)) {
            throw new makeTypeError("connect takes one options object");
        }
        const keys = objectKeys(options);
        for (let index = 0; index < keys.length; index += 1) {
            const key = keys[index];
            if (!arrayIncludes(SSH_OPTION_KEYS, key)) {
                throw new makeTypeError(`unknown SSH option \`${key}\``);
            }
        }
        const response = opSshConnect(
            options.host,
            options.port,
            options.username,
            options.password,
            options.privateKey,
            options.agent,
            options.passphrase,
            options.hostKey,
            options.knownHosts,
            options.connectTimeoutMs,
            options.execTimeoutMs,
        );
        if (!response.ok) throwSsh(response);
        if (typeof response.id !== "number") {
            throw sshError(
                "VAEL_SSH_INTERNAL",
                "the SSH module returned an invalid connection handle",
            );
        }
        return new SshConnection(sshConnectionToken, response.id);
    }

    // A frozen module object, like `chars` and `crypto`: the properties cannot
    // be replaced, but `connect` is shared with the imports. Host trust and
    // credentials are per-call arguments; this module stores no keys.
    const ssh = Object.freeze({
        connect: sshConnect,
        MAX_COMMAND_BYTES,
        MAX_OUTPUT_BYTES,
        DEFAULT_CONNECT_TIMEOUT_MS,
        DEFAULT_EXEC_TIMEOUT_MS,
    });

    // The largest number of bytes one memory-backed stream retains, and the
    // largest single writeSync chunk. It must match `MAX_BYTES` in
    // `vaeljs-core/src/stdio.rs`; the boundary tests exercise both sides.
    const MAX_BYTES = 8388608;

    /**
     * Create the ordinary `Error` this module reports, with a stable own
     * `code` and `name = "StdioError"`. `code` is defined rather than assigned
     * so an inherited `Object.prototype.code` accessor cannot swallow it.
     */
    function stdioError(code, message) {
        const error = new makeError(message);
        defineProperty(error, "name", {
            __proto__: null,
            value: "StdioError",
            writable: true,
            enumerable: false,
            configurable: true,
        });
        defineProperty(error, "code", {
            __proto__: null,
            value: code,
            writable: true,
            enumerable: true,
            configurable: true,
        });
        return error;
    }

    /**
     * Return the host result object on success, or throw the documented
     * exception. A `TypeError` result is a wrong type; everything else is a
     * `StdioError` with a stable code and a message that never includes the
     * rejected bytes.
     */
    function stdioResult(response) {
        if (response.ok) return response;
        if (response.name === "TypeError") {
            throw new makeTypeError(response.message);
        }
        throw stdioError(response.code, response.message);
    }

    /**
     * A synchronous reader over an owned snapshot of a `Uint8Array`.
     *
     * The input view is copied at construction, so mutating, detaching, or
     * resizing the caller's buffer afterwards does not change the reader.
     * `readSync` copies at most `target.byteLength` bytes and returns the count,
     * or exactly `0` at end of file. It can return less than the target holds,
     * so loop until it returns `0`. An empty target throws
     * `VAEL_STDIO_ARGUMENT` rather than returning an ambiguous zero. `close()`
     * releases the snapshot, is idempotent, and never detaches a caller's
     * buffer; reads after it throw `VAEL_STDIO_CLOSED`.
     */
    class Readable {
        #bytes;
        #length = 0;
        #offset = 0;
        #closed = false;

        /**
         * Copy the current byte range of `bytes` into reader-owned storage.
         *
         * @param {Uint8Array} bytes an attached view of at most MAX_BYTES
         * @throws {TypeError} when the argument count is not one or the value
         * is not a `Uint8Array`.
         * @throws {Error} with `code` `VAEL_STDIO_DETACHED` or
         * `VAEL_STDIO_LIMIT`.
         */
        constructor(bytes) {
            if (arguments.length !== 1) {
                throw new makeTypeError("Readable takes exactly one argument");
            }
            if (typeof bytes === "string") {
                throw new makeTypeError("Readable takes a Uint8Array, not a string");
            }
            const copy = stdioResult(opStdioCopy(bytes, MAX_BYTES));
            this.#bytes = copy.value;
            this.#length = copy.length;
        }

        /** Bytes not yet read; `0` at end of file and after `close()`. */
        get remaining() {
            return this.#closed ? 0 : this.#length - this.#offset;
        }

        /** Whether `close()` has been called. */
        get closed() {
            return this.#closed;
        }

        /**
         * Copy up to `target.byteLength` remaining bytes into `target`.
         *
         * @param {Uint8Array} target an attached, non-empty view
         * @returns {number} the bytes copied, or `0` at end of file
         * @throws {TypeError} when the argument count is not one or the value
         * is not a `Uint8Array`.
         * @throws {Error} with `code` `VAEL_STDIO_CLOSED` (before the target is
         * validated), `VAEL_STDIO_DETACHED`, or `VAEL_STDIO_ARGUMENT` for an
         * empty target, even at end of file.
         */
        readSync(target) {
            if (arguments.length !== 1) {
                throw new makeTypeError("readSync takes exactly one argument");
            }
            if (this.#closed) {
                throw stdioError("VAEL_STDIO_CLOSED", "the reader is closed");
            }
            const targetLength = stdioResult(opStdioViewLength(target)).value;
            if (targetLength === 0) {
                throw stdioError("VAEL_STDIO_ARGUMENT", "readSync target must not be empty");
            }
            const available = this.#length - this.#offset;
            if (available === 0) return 0;
            const count = targetLength < available ? targetLength : available;
            // The native slice op copies the range. Do not use `subarray`: its
            // `Symbol.species` lookup would run user JavaScript with the
            // private snapshot buffer and allow reentrancy.
            const slice = stdioResult(opStdioSlice(this.#bytes, this.#offset, this.#offset + count));
            typedArraySet(target, slice.value, 0);
            this.#offset += slice.length;
            return slice.length;
        }

        /** Release the owned snapshot. Idempotent. */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            this.#bytes = null;
            this.#length = 0;
            this.#offset = 0;
        }
    }

    // Assigned by the static block below. It is private to this shim, so user
    // code cannot construct a host-backed writer.
    let hostWritable;

    /**
     * A synchronous writer. `new Writable()` creates a memory-backed writer
     * over one coalesced, owned buffer whose capacity is bounded by MAX_BYTES;
     * empty writes retain nothing and there is no per-write record. The exported
     * `stdout` and `stderr` are host-backed writers for the current process's
     * standard streams.
     *
     * `writeSync` accepts a `Uint8Array` or a string encoded as UTF-8 and
     * returns the number of bytes accepted. A memory writer is all-or-nothing:
     * it appends the whole chunk or throws without changing the stream. A host
     * writer blocks until the operating system accepts every byte; a throw can
     * mean that a prefix was already written, and there is no retry or
     * rollback. `end()` retires the writer but keeps memory-backed content
     * readable; `close()` is terminal and releases it. Neither closes the
     * process descriptor. Both are idempotent. `stdout` and `stderr` are shared
     * per realm, so retiring one through any reference is visible through every
     * reference; `console` does not go through these objects.
     */
    class Writable {
        #kind;
        #stream;
        // Memory-backed writers keep one coalesced buffer. Its capacity is
        // bounded by MAX_BYTES and never grows because of empty writes; there
        // is no per-write record that could grow without bound.
        #buffer;
        #capacity;
        #length;
        #closed;
        #ended;

        /**
         * Create an empty memory-backed writer.
         *
         * @throws {TypeError} when any argument is supplied.
         */
        constructor() {
            if (arguments.length !== 0) {
                throw new makeTypeError("Writable takes no arguments");
            }
            this.#kind = "memory";
            this.#stream = 0;
            this.#buffer = null;
            this.#capacity = 0;
            this.#length = 0;
            this.#closed = false;
            this.#ended = false;
        }

        static {
            // A class static block can install private state, so the host
            // writers are created without exposing a factory or constructor
            // form to user code.
            hostWritable = (stream, kind) => {
                const writer = new Writable();
                writer.#stream = stream;
                writer.#kind = kind;
                return writer;
            };
        }

        /**
         * Grow the coalesced buffer to hold at least `required` bytes.
         *
         * Capacity doubles from 1 KiB, is clamped to MAX_BYTES, and is only
         * allocated when a non-empty append needs room. The old contents are
         * copied before the new buffer is installed, so a failed allocation
         * leaves the writer unchanged. `#length` never includes unused
         * capacity, and `toUint8Array` copies only the used prefix.
         */
        #reserve(required) {
            if (required <= this.#capacity) return;
            let capacity = this.#capacity === 0 ? 1024 : this.#capacity * 2;
            if (capacity < required) capacity = required;
            if (capacity > MAX_BYTES) capacity = MAX_BYTES;
            if (capacity < required) {
                throw stdioError("VAEL_STDIO_LIMIT", "byte data exceeds the stdio size limit");
            }
            const buffer = new makeUint8Array(capacity);
            if (this.#length > 0) typedArraySet(buffer, this.#buffer, 0);
            this.#buffer = buffer;
            this.#capacity = capacity;
        }

        /** `"memory"`, `"stdout"`, or `"stderr"`. */
        get kind() {
            return this.#kind;
        }

        /**
         * Bytes retained by a memory writer, or `0` for a host writer, which
         * retains nothing. `0` after `close()`.
         */
        get length() {
            return this.#closed ? 0 : this.#length;
        }

        /** Whether `close()` has been called. */
        get closed() {
            return this.#closed;
        }

        /** Whether `end()` or `close()` has been called. */
        get ended() {
            return this.#ended;
        }

        /**
         * Append a chunk, or write it to the host stream.
         *
         * @param {Uint8Array | string} chunk at most MAX_BYTES as UTF-8 bytes
         * @returns {number} the bytes accepted, never a short count
         * @throws {TypeError} when the argument count is not one or the value
         * is neither a `Uint8Array` nor a string.
         * @throws {Error} with `code` `VAEL_STDIO_CLOSED`, `VAEL_STDIO_ENDED`,
         * `VAEL_STDIO_DETACHED`, `VAEL_STDIO_LIMIT`, or `VAEL_STDIO_WRITE`.
         */
        writeSync(chunk) {
            if (arguments.length !== 1) {
                throw new makeTypeError("writeSync takes exactly one argument");
            }
            if (this.#closed) {
                throw stdioError("VAEL_STDIO_CLOSED", "the writer is closed");
            }
            if (this.#ended) {
                throw stdioError("VAEL_STDIO_ENDED", "the writer has ended");
            }
            if (this.#kind !== "memory") {
                return stdioResult(opStdioWrite(this.#stream, chunk)).value;
            }
            const capacity = MAX_BYTES - this.#length;
            const copy = stdioResult(opStdioCopy(chunk, capacity));
            if (copy.length === 0) return 0;
            this.#reserve(this.#length + copy.length);
            typedArraySet(this.#buffer, copy.value, this.#length);
            this.#length += copy.length;
            return copy.length;
        }

        /** Stop accepting writes. Idempotent; does not release content. */
        end() {
            if (arguments.length !== 0) {
                throw new makeTypeError("end takes no arguments");
            }
            if (this.#closed) return;
            this.#ended = true;
        }

        /**
         * Return a fresh `Uint8Array` copy of the memory-backed content. The
         * caller owns the result; changing it does not change the writer.
         *
         * @throws {Error} with `code` `VAEL_STDIO_CLOSED`, or
         * `VAEL_STDIO_NOT_MEMORY` when the writer is `stdout` or `stderr`.
         */
        toUint8Array() {
            if (arguments.length !== 0) {
                throw new makeTypeError("toUint8Array takes no arguments");
            }
            if (this.#closed) {
                throw stdioError("VAEL_STDIO_CLOSED", "the writer is closed");
            }
            if (this.#kind !== "memory") {
                throw stdioError("VAEL_STDIO_NOT_MEMORY", "the stream is not memory-backed");
            }
            if (this.#length === 0) return new makeUint8Array(0);
            // The native slice op copies only the used prefix and returns a
            // fresh caller-owned array. It does not consult species.
            return stdioResult(opStdioSlice(this.#buffer, 0, this.#length)).value;
        }

        /** Release memory-backed content and retire the writer. Idempotent. */
        close() {
            if (arguments.length !== 0) {
                throw new makeTypeError("close takes no arguments");
            }
            if (this.#closed) return;
            this.#closed = true;
            this.#ended = true;
            if (this.#kind === "memory") {
                this.#buffer = null;
                this.#capacity = 0;
                this.#length = 0;
            }
        }
    }

    // The two host writers are created once per realm and shared by the
    // registry and every import. They use the current process's own standard
    // descriptors; closing one retires the wrapper and leaves the descriptor
    // open for console and for the other references to this same object.
    const stdout = hostWritable(1, "stdout");
    const stderr = hostWritable(2, "stderr");

    // A frozen module object, like `chars`, `crypto`, and `age`: the properties
    // cannot be replaced, but the classes and the shared stream objects are the
    // same values every importer sees.
    const stdio = Object.freeze({
        Readable,
        Writable,
        stdout,
        stderr,
        MAX_BYTES,
    });

    // The console module is the realm's installed console object. The console
    // shim runs before this script, so the registry, the module, and the global
    // share the exact function objects at installation time. It is deliberately
    // not frozen: it is the live console object, and replacing a method stays
    // visible through every path that holds the object. That is a mutation
    // contract, not a security boundary.
    const consoleModule = globalThis.console;

    // The `vael:os` built-in module object.
    //
    // `platform` uses Deno's `Deno.build.os` target vocabulary (`darwin`,
    // `linux`, `windows`, etc.), and `arch` uses normalized OCI/Go-style short
    // names (`amd64`, `arm64`, `386`, etc., not Rust's `x86_64`/`aarch64` or
    // Node's `x64`/`ia32`) for the target this runtime was compiled for. The two
    // placeholders are replaced by `vaeljs-core/src/builtins.rs` before this
    // script is evaluated; they are compilation-target constants, not a runtime
    // probe. macOS is `darwin` (not Rust's `macos`).
    //
    // This module calls no host op. It reads no environment variable,
    // OS-release file, registry, sysctl value, file, or process table, and it
    // installs no global. `version`, `codename`, `variant`, user, keyring, and
    // scheduler surfaces are not implemented here.
    // The `vael:cron` built-in module object.
    //
    // This is a UTC five-field cron parser and an in-process scheduler. The
    // grammar, the POSIX day-of-month/day-of-week rule, the 100-year occurrence
    // search, the scheduler's overlap and missed-tick rules, and the liveness
    // contract are documented in `vaeljs-core/src/cron.rs`. It is not Deno's
    // `Deno.cron`, Bun's `Bun.cron`, `node-cron`, or any other implementation's
    // compatibility layer.
    //
    // `cron(expression, callback, options?)` returns a frozen controller.
    // `dispose()` is idempotent and is also installed as `[Symbol.dispose]`
    // when the engine provides that symbol. `next(expression, after?)` returns
    // a `Date` or `null`. The host ops are captured once, so replacing
    // `Deno.core.ops` later does not change what this module calls.
    //
    // Scheduling: one callback runs at a time. A tick that arrives while the
    // previous callback is still running is skipped and reported as `overlap`.
    // A wake-up so late that the following occurrence is already due is
    // reported as `missed`; the stale occurrence is not run, and the schedule
    // continues from the current time. When `due` is the last occurrence the
    // `Date` range can represent, it is dispatched once and the job then stops
    // and releases its host resource, unless the preceding callback is still in
    // flight: the normal overlap rule then skips that occurrence and reports
    // `overlap`, and the job still stops and releases. The host wait re-reads
    // the wall clock at
    // least once a minute and resolves only once the clock reaches the due
    // time; the scheduler also re-reads it before dispatch, so a backward clock
    // adjustment postpones a tick instead of running it early. A forward jump
    // can still make a tick late, which the `missed` rule then discards rather
    // than replaying a backlog. A callback's thrown value or rejected Promise
    // is reported as `error` and does not stop the schedule. With no `onEvent`
    // observer, callback failures are thrown from a microtask so they are not
    // silently lost; an exception thrown by `onEvent` itself is discarded to
    // keep the schedule alive.
    //
    // An undisposed job keeps the isolate alive: its pending wait op is
    // event-loop work. `dispose()` cancels the pending wait, but an already
    // started callback is not interrupted. Dropping the runtime abandons all
    // jobs. The scheduler uses the runtime's Tokio timer, not `setTimeout`, so a
    // callback that blocks the isolate delays every timer. There is no fake
    // timer surface: the deterministic test clock is a Rust-only test-support
    // hook and is absent from a normal build.
    const cron = (() => {
        const cronOps = Deno.core.ops;
        const opCreate = cronOps.op_vael_cron_create;
        const opDispose = cronOps.op_vael_cron_dispose;
        const opNext = cronOps.op_vael_cron_next;
        const opFollowing = cronOps.op_vael_cron_following;
        const opWait = cronOps.op_vael_cron_wait;
        const opNow = cronOps.op_vael_cron_now;

        const cronError = Error;
        const cronTypeError = TypeError;
        const cronRangeError = RangeError;
        const cronDate = Date;
        const cronGetTime = Function.prototype.call.bind(Date.prototype.getTime);
        // `Promise.resolve` needs `this` to be the `Promise` constructor, so it
        // cannot be called unbound. Bind it once, like the other intrinsics.
        const cronPromise = Promise;
        const cronResolve = Promise.resolve.bind(cronPromise);
        const cronQueueMicrotask = queueMicrotask;
        const cronFreeze = Object.freeze;
        const cronDefineProperty = Object.defineProperty;
        const cronObjectKeys = Object.keys;
        const cronIsFinite = Number.isFinite;
        const cronIsNaN = Number.isNaN;
        const cronDispose =
            typeof Symbol.dispose === "symbol" ? Symbol.dispose : null;

        /**
         * A cron expression, limit, no-occurrence, or argument failure.
         *
         * `code` is one of `VAEL_CRON_EXPRESSION`, `VAEL_CRON_LIMIT`,
         * `VAEL_CRON_NO_OCCURRENCE`, or `VAEL_CRON_ARGUMENT`. The message is for
         * people and may change; match on the code.
         */
        class CronError extends cronError {
            constructor(code, message) {
                super(message);
                this.name = "CronError";
                this.code = code;
            }
        }

        function failure(outcome) {
            return new CronError(outcome.code, outcome.message);
        }

        function requireExpression(expression) {
            if (typeof expression !== "string") {
                throw new cronTypeError("cron expression must be a string");
            }
        }

        /**
         * The first occurrence strictly after `after`, as a `Date`, or `null`.
         *
         * `after` is a `Date` or an epoch millisecond number and defaults to
         * the host clock. Occurrences are UTC and have zero seconds and
         * milliseconds. An invalid expression throws `CronError`; a valid
         * expression with no occurrence in the 100-year search window, or none
         * within the `Date` range, returns `null`. This is a pure query: it
         * creates no job and schedules nothing.
         */
        function next(expression, after) {
            requireExpression(expression);
            let at;
            if (after === undefined) {
                at = opNow();
            } else if (after instanceof cronDate) {
                at = cronGetTime(after);
                if (!cronIsFinite(at)) {
                    throw new cronRangeError("after is an invalid Date");
                }
            } else if (typeof after === "number") {
                if (!cronIsFinite(after)) {
                    throw new cronRangeError("after must be a finite epoch millisecond number");
                }
                at = after;
            } else {
                throw new cronTypeError("after must be a Date or an epoch millisecond number");
            }
            const outcome = opNext(expression, at);
            if (!outcome.ok) throw failure(outcome);
            return outcome.value === null ? null : new cronDate(outcome.value);
        }

        /**
         * Run `callback` on a UTC cron schedule.
         *
         * The first tick is strictly after this call. The returned controller is
         * frozen; `dispose()` is idempotent and cancels the pending wait.
         * `options.onEvent(event)` receives `{ type: "overlap" | "missed" |
         * "error", ... }` events. The callback is called with no arguments; a
         * returned Promise is awaited before the next tick is considered
         * available. If the next occurrence is the last minute the `Date` range
         * represents, it runs once and the job then stops and releases its host
         * resource. If the preceding callback is still in flight, the overlap
         * rule skips that final occurrence and reports `overlap` instead; the
         * job still stops and releases. Throws `TypeError` for bad arguments
         * and `CronError` for an invalid expression or one with no future
         * occurrence.
         */
        function cronJob(expression, callback, options) {
            requireExpression(expression);
            if (typeof callback !== "function") {
                throw new cronTypeError("cron callback must be a function");
            }
            let onEvent = null;
            if (options !== undefined) {
                if (options === null || typeof options !== "object") {
                    throw new cronTypeError("cron options must be an object");
                }
                const keys = cronObjectKeys(options);
                for (let index = 0; index < keys.length; index += 1) {
                    if (keys[index] !== "onEvent") {
                        throw new cronTypeError(`unknown cron option: ${keys[index]}`);
                    }
                }
                if (options.onEvent !== undefined) {
                    if (typeof options.onEvent !== "function") {
                        throw new cronTypeError("cron option onEvent must be a function");
                    }
                    onEvent = options.onEvent;
                }
            }

            const created = opCreate(expression);
            if (!created.ok) throw failure(created);
            const token = created.token;
            let disposed = false;
            let inFlight = false;

            function emit(event) {
                if (onEvent === null) return;
                try {
                    onEvent(cronFreeze(event));
                } catch {
                    // An observer that throws must not stop the schedule. The
                    // module does not log; a failing observer is the caller's
                    // diagnostic to make.
                }
            }

            function reportError(error, fatal) {
                if (onEvent !== null) {
                    emit({ type: "error", error, fatal });
                    return;
                }
                // No observer: surface the failure as an uncaught exception
                // instead of discarding it. `queueMicrotask` routes through the
                // engine's report-exception callback.
                cronQueueMicrotask(() => {
                    throw error;
                });
            }

            function dispose() {
                if (disposed) return;
                disposed = true;
                opDispose(token);
            }

            const job = { dispose };
            if (cronDispose !== null) {
                cronDefineProperty(job, cronDispose, {
                    value: dispose,
                    enumerable: false,
                    writable: false,
                    configurable: false,
                });
            }
            cronFreeze(job);

            (async () => {
                let due = created.next;
                try {
                    while (!disposed) {
                        const woke = await opWait(token, due);
                        if (disposed || woke === -1 || cronIsNaN(woke)) break;
                        due = woke;
                        const now = opNow();

                        // The wall clock can move backward between the wait
                        // resolving and this read. Waiting again for the same
                        // occurrence is the only safe response: dispatching
                        // here would run it before its due time.
                        if (now < due) continue;

                        if (inFlight) {
                            emit({
                                type: "overlap",
                                scheduledTime: new cronDate(due),
                                wokeAt: new cronDate(now),
                            });
                            due = opFollowing(token, now);
                            if (cronIsNaN(due)) break;
                            continue;
                        }

                        // No following occurrence means `due` is the last
                        // representable one. It is still due and must run;
                        // the lookup after the callback stops the loop and
                        // releases the job.
                        const following = opFollowing(token, due);
                        if (!cronIsNaN(following) && following <= now) {
                            emit({
                                type: "missed",
                                scheduledTime: new cronDate(due),
                                wokeAt: new cronDate(now),
                            });
                            due = opFollowing(token, now);
                            if (cronIsNaN(due)) break;
                            continue;
                        }

                        inFlight = true;
                        try {
                            const result = callback();
                            let then;
                            if (
                                result !== null &&
                                (typeof result === "object" || typeof result === "function")
                            ) {
                                then = result.then;
                            }
                            if (typeof then === "function") {
                                cronResolve(result).then(
                                    () => {
                                        inFlight = false;
                                    },
                                    (error) => {
                                        inFlight = false;
                                        reportError(error, false);
                                    },
                                );
                            } else {
                                inFlight = false;
                            }
                        } catch (error) {
                            inFlight = false;
                            reportError(error, false);
                        }

                        due = opFollowing(token, due);
                        if (cronIsNaN(due)) break;
                    }
                } catch (error) {
                    reportError(error, true);
                } finally {
                    dispose();
                }
            })();

            return job;
        }

        return cronFreeze({
            cron: cronJob,
            next,
            CronError,
        });
    })();

    const os = Object.freeze({
        platform: __VAEL_OS_PLATFORM__,
        arch: __VAEL_OS_ARCH__,
    });

    // The `vael:path` built-in module object.
    //
    // This is a lexical path implementation adapted from NeoLiteJS `neo:path` and
    // `@neotales/path`. See `vaeljs-core/THIRD_PARTY_NOTICES.md` for the exact
    // upstream revision, the adapted files, and the MIT notices. It is not a
    // drop-in replacement for Node's `path`, `@neotales/path`, or NeoLiteJS
    // `neo:path`.
    //
    // Scope: POSIX and Windows lexical operations only. The module does not read or
    // write files, check existence, resolve symlinks or junctions, read environment
    // variables, normalize Unicode, or validate Windows device names or alternate
    // data streams. It is not a containment check. The only host read is the
    // process working directory, through `opVaelPathCwd`, and only for
    // `resolve`/`relative`/`toNamespacedPath` when their inputs do not supply a
    // complete base. Glob and file URL helpers are deliberately absent.
    //
    // The two namespaces are frozen and available on every host. The named exports
    // select the build host's dialect from `opVaelPathIsWindows`, never from a
    // script-replaceable global. Every path argument must be a primitive string;
    // nothing is coerced. A NUL is rejected because it cannot appear in a usable
    // C path. Failures needing a working directory are `PathError` with an own
    // `code`: `PATH_CWD`, `PATH_CWD_DIALECT`, or `PATH_WINDOWS_ROOT`.
    //
    // `common` compares components as written and does not normalize or fold
    // case. On Windows it is the documented exception to the two-separator
    // input rule: it splits on `\` only, matching the reference, so `/` is not
    // a component boundary there.
    //
    // `relative` resolves its two endpoints against one lazy per-call cwd
    // snapshot. It reads the process cwd at most once per call, and not at all
    // when both endpoints already have a complete absolute base. `resolve` and
    // Windows `toNamespacedPath` likewise read at most once per call.
    //
    // This block is evaluated inside the `vael.js` IIFE before user code. It is
    // wrapped in its own scope so its captured intrinsics do not collide with the
    // other built-in modules.
    const pathOps = Deno.core.ops;
    const path = (
        // __VAEL_PATH_IMPL__
    )({
        // Capture the op functions once before user code runs; replacing a
        // property on `Deno.core.ops` later must not change what path calls.
        isWindows: pathOps.op_vael_path_is_windows(),
        readCwd: pathOps.op_vael_path_cwd,
    });

    const assert = (
        // __VAEL_ASSERT_IMPL__
    )({
        isNativePromise: Deno.core.isPromise,
        observePromise: cryptoOps.op_vael_assert_observe_promise,
    });
    // __VAEL_SLICES_IMPL__
    // __VAEL_DB_IMPL__
    // __VAEL_EXEC_IMPL__
    // __VAEL_FETCH_IMPL__
    // __VAEL_TEMPLATE_IMPL__

    const testCore = Deno.core;
    const testHarness = (
        // __VAEL_TEST_IMPL__
    )({
        isNativePromise: Deno.core.isPromise,
        observePromise: cryptoOps.op_vael_assert_observe_promise,
        print: testCore.print,
        createTimer: testCore.createTimer,
        cancelTimer: testCore.cancelTimer,
        setUnhandledRejectionHandler: testCore.setUnhandledPromiseRejectionHandler,
        eventLoopHasMoreWork: testCore.eventLoopHasMoreWork,
    });
    const testModule = testHarness.module;
    const testEnsureHandler = testHarness.ensureHandler;

    // __VAEL_HTTP_IMPL__

    // The key is the canonical `vael:` specifier the loader serves. A bare
    // registry name is never a key: bare imports are not built-in modules.
    const modules = new Map([
        ["vael:age", age],
        ["vael:assert", assert],
        // __VAEL_NATIVE_ENTRIES__
        ["vael:console", consoleModule],
        ["vael:cron", cron],
        ["vael:crypto", crypto],
        // __VAEL_DB_REGISTRY_ENTRY__
        // __VAEL_EXEC_REGISTRY_ENTRY__
        // __VAEL_FETCH_REGISTRY_ENTRY__
        ["vael:fs", fs],
        // __VAEL_HTTP_REGISTRY_ENTRY__
        ["vael:os", os],
        ["vael:path", path],
        // __VAEL_SLICES_REGISTRY_ENTRY__
        ["vael:ssh", ssh],
        ["vael:stdio", stdio],
        // __VAEL_NATIVE_STRINGS_ENTRY__
        // __VAEL_TEMPLATE_REGISTRY_ENTRY__
        ["vael:test", testModule],
        ["vael:uuid", uuid],
        ["vael:uuid/v4", uuidV4Module],
        ["vael:uuid/v7", uuidV7Module],
    ]);

    // One canonical `vael:` specifier per built-in, in the registry's insertion
    // order. Building this from `modules` keeps the public list aligned with
    // what `getBuiltInModule` and the loader actually implement, including the
    // `vael:uuid/v4` and `vael:uuid/v7` subpaths.
    const builtInModuleList = [];
    mapForEachEntry(modules, (_module, specifier) => {
        builtInModuleList.push(specifier);
    });
    freeze(builtInModuleList);

    /**
     * Return a built-in module by its canonical `vael:` specifier, or
     * `undefined` when the specifier is not implemented. The same module
     * object is returned for every lookup. A bare registry name such as
     * `"age"` is not a key and returns `undefined`.
     *
     * @throws {TypeError} when the name is not a string.
     */
    function getBuiltInModule(name) {
        if (typeof name !== "string") {
            throw new makeTypeError("module name must be a string");
        }
        if (name === "vael:test") testEnsureHandler();
        return mapGet(modules, name);
    }

    /**
     * Return every implemented built-in module specifier as one frozen array.
     *
     * Entries are in registration order and are the canonical `vael:`
     * specifiers only, including the `uuid/v4` and `uuid/v7` subpaths. The
     * array is frozen and the same array is returned for every call, so a
     * caller cannot change later results. It lists no bare names and no Node
     * modules: bare imports are not built-in modules and a `node:` opt-in is
     * not implemented.
     */
    function getBuiltInModulesList() {
        return builtInModuleList;
    }

    // The engine name is `vael` for this V8 runtime. The QuickJS runtime reports
    // `vael-lite` with an empty built-in module list.
    const engine = "vael";

    Object.defineProperty(globalThis, "Vael", {
        value: Object.freeze({ engine, getBuiltInModule, getBuiltInModulesList }),
        writable: false,
        enumerable: false,
        configurable: false,
    });
})();
