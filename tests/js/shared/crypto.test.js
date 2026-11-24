import test from "neo:test";
import assert from "neo:assert";
import * as cryptoNs from "neo:crypto";
import {
    bcryptHash,
    bcryptVerify,
    createDecryptor,
    createEncryptor,
    createHash,
    createHmac,
    decrypt,
    encrypt,
    generateEnvelopeKey,
    pbkdf2,
    randomBytes,
    shamirCombine,
    shamirSplit,
} from "neo:crypto";

const SYNC_EXPORTS = [
    "bcryptHash",
    "bcryptVerify",
    "createDecryptor",
    "createEncryptor",
    "createHash",
    "createHmac",
    "decrypt",
    "encrypt",
    "generateEnvelopeKey",
    "pbkdf2",
    "randomBytes",
    "shamirCombine",
    "shamirSplit",
];

const ASYNC_EXPORTS = [
    "bcryptHashAsync",
    "bcryptVerifyAsync",
    "hashAsync",
    "hmacAsync",
    "pbkdf2Async",
    "shamirCombineAsync",
    "shamirSplitAsync",
];

function fromHex(text) {
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i += 1) {
        out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}

function toHex(bytes) {
    let out = "";
    for (let i = 0; i < bytes.length; i += 1) {
        out += bytes[i].toString(16).padStart(2, "0");
    }
    return out;
}

function expectCryptoError(fn, expectedCode) {
    let caught = null;
    try {
        fn();
    } catch (error) {
        caught = error;
    }
    assert.ok(caught instanceof Error, `expected Error (${expectedCode})`);
    assert.strictEquals(caught.code, expectedCode);
}

test("neo:crypto exposes frozen module and identical synchronous function references", async () => {
    const list = Neo.getBuiltInModulesList();
    assert.ok(list.includes("neo:crypto"), "neo:crypto missing from Neo.getBuiltInModulesList()");
    assert.ok(!list.includes("crypto"), "bare crypto must not be listed");
    assert.strictEquals(Neo.getBuiltInModule("crypto"), undefined);

    const reg = Neo.getBuiltInModule("neo:crypto");
    assert.ok(reg !== undefined && typeof reg === "object" && reg !== null);
    assert.strictEquals(Neo.getBuiltInModule("neo:crypto"), reg);
    assert.strictEquals(Object.isFrozen(reg), true);
    assert.ok(cryptoNs !== reg, "module namespace must be distinct from registry");

    const dyn = await import("neo:crypto");
    assert.strictEquals(dyn, cryptoNs);
    assert.ok(!("default" in cryptoNs), "neo:crypto must not have a default export");
    assert.ok(!("generateKey" in reg) && !("generateKey" in cryptoNs));

    const namedMap = {
        bcryptHash,
        bcryptVerify,
        createDecryptor,
        createEncryptor,
        createHash,
        createHmac,
        decrypt,
        encrypt,
        generateEnvelopeKey,
        pbkdf2,
        randomBytes,
        shamirCombine,
        shamirSplit,
    };

    for (const name of SYNC_EXPORTS) {
        assert.strictEquals(typeof reg[name], "function", `${name} missing`);
        assert.strictEquals(cryptoNs[name], reg[name], `${name} namespace identity mismatch`);
        assert.strictEquals(namedMap[name], reg[name], `${name} named import identity mismatch`);
    }

    if (Neo.engine === "neo-lite") {
        assert.equals(Object.keys(reg).sort(), SYNC_EXPORTS);
        assert.equals(Object.keys(cryptoNs).sort(), SYNC_EXPORTS);
        for (const asyncName of ASYNC_EXPORTS) {
            assert.ok(!(asyncName in reg), `${asyncName} must not be on neo-lite registry`);
            assert.ok(!(asyncName in cryptoNs), `${asyncName} must not be on neo-lite namespace`);
        }
    }
});

test("generateEnvelopeKey, randomBytes, encrypt, and decrypt roundtrip and reject tampering", () => {
    const key = generateEnvelopeKey();
    assert.ok(key instanceof Uint8Array);
    assert.strictEquals(key.length, 32);
    assert.strictEquals(typeof key.then, "undefined");

    const r0 = randomBytes(0);
    assert.ok(r0 instanceof Uint8Array && r0.length === 0);
    const rNeg0 = randomBytes(-0);
    assert.strictEquals(rNeg0.length, 0);
    const r32a = randomBytes(32);
    const r32b = randomBytes(32);
    assert.strictEquals(r32a.length, 32);
    assert.notEquals(toHex(r32a), toHex(r32b));

    const plaintext = Uint8Array.from([0, 1, 2, 0x80, 0xff, 42]);
    const aad = Uint8Array.from([0x61, 0x61, 0x64]);
    const env = encrypt(key, plaintext, aad);
    assert.ok(env instanceof Uint8Array);
    assert.strictEquals(env.length, plaintext.length + 34);

    const out = decrypt(key, env, aad);
    assert.equals(Array.from(out), Array.from(plaintext));
    assert.ok(out.buffer !== plaintext.buffer && out.buffer !== env.buffer);

    // Tampered tag, wrong AAD, wrong key, malformed header, bad key length
    const badTag = env.slice();
    badTag[badTag.length - 1] ^= 0x01;
    expectCryptoError(() => decrypt(key, badTag, aad), "NEO_CRYPTO_AUTH");
    expectCryptoError(() => decrypt(key, env, Uint8Array.from([1])), "NEO_CRYPTO_AUTH");
    expectCryptoError(() => decrypt(generateEnvelopeKey(), env, aad), "NEO_CRYPTO_AUTH");
    expectCryptoError(() => decrypt(key, env.slice(0, 10)), "NEO_CRYPTO_FORMAT");
    expectCryptoError(() => encrypt(new Uint8Array(31), plaintext), "NEO_CRYPTO_KEY");
    expectCryptoError(() => decrypt(new Uint8Array(31), env), "NEO_CRYPTO_KEY");

    // Validation errors
    assert.throws(() => generateEnvelopeKey(1), TypeError);
    assert.throws(() => randomBytes(), TypeError);
    assert.throws(() => randomBytes("4"), TypeError);
    assert.throws(() => randomBytes(-1), RangeError);
    assert.throws(() => randomBytes(1.5), RangeError);
    assert.throws(() => randomBytes(Number.NaN), RangeError);
    assert.throws(() => randomBytes(8 * 1024 * 1024 + 1), RangeError);
    assert.throws(() => encrypt(key), TypeError);
    assert.throws(() => encrypt(key, "not-u8"), TypeError);
    assert.throws(() => decrypt(key, env, "not-u8"), TypeError);
});

test("createEncryptor and createDecryptor support AES-256-CBC and AES-256-GCM", () => {
    const cbcKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
    const cbcIv = fromHex("101112131415161718191a1b1c1d1e1f");
    const cbcPlain = fromHex("6e656f6a73206369706865722063626320696e7465726f70");
    const cbcExpected = "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85";

    const enc = createEncryptor("aes-256-cbc", cbcKey, cbcIv);
    try {
        const up = enc.update(cbcPlain.subarray(0, 5));
        assert.ok(up instanceof Uint8Array && up.length === 0);
        enc.update(cbcPlain.subarray(5));
        const ct = enc.final();
        assert.strictEquals(toHex(ct), cbcExpected);
        expectCryptoError(() => enc.update(new Uint8Array(1)), "NEO_CRYPTO_STATE");
        expectCryptoError(() => enc.final(), "NEO_CRYPTO_STATE");
        expectCryptoError(() => enc.setAAD(new Uint8Array(1)), "NEO_CRYPTO_STATE");
    } finally {
        enc.close();
        enc.dispose();
    }
    expectCryptoError(() => enc.final(), "NEO_CRYPTO_CLOSED");

    const dec = createDecryptor("AES-256-CBC", cbcKey, cbcIv);
    try {
        dec.update(fromHex(cbcExpected));
        assert.strictEquals(toHex(dec.final()), toHex(cbcPlain));
    } finally {
        dec.close();
    }

    // CBC no-padding option
    const block = new Uint8Array(16).fill(0x41);
    const npEnc = createEncryptor("aes-256-cbc", cbcKey, cbcIv, { padding: false });
    let npCt;
    try {
        npEnc.update(block);
        npCt = npEnc.final();
        assert.strictEquals(npCt.length, 16);
    } finally {
        npEnc.close();
    }
    const npDec = createDecryptor("aes-256-cbc", cbcKey, cbcIv);
    try {
        npDec.setAutoPadding(false);
        npDec.update(npCt);
        assert.equals(Array.from(npDec.final()), Array.from(block));
    } finally {
        npDec.close();
    }

    // AES-256-GCM with AAD and authTag
    const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
    const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
    const gcmPlain = fromHex("6e656f6a73206369706865722067636d20696e7465726f70");
    const gcmExpectedCt = "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1";
    const gcmExpectedTag = "9ea6507bfbc87455b1f24edc000cc662";

    const gcmEnc = createEncryptor("aes-256-gcm", cbcKey, gcmIv);
    let gcmCt;
    let gcmTag;
    try {
        assert.strictEquals(gcmEnc.setAAD(gcmAad), gcmEnc);
        expectCryptoError(() => gcmEnc.getAuthTag(), "NEO_CRYPTO_STATE");
        gcmEnc.update(gcmPlain);
        gcmCt = gcmEnc.final();
        gcmTag = gcmEnc.getAuthTag();
        assert.strictEquals(toHex(gcmCt), gcmExpectedCt);
        assert.strictEquals(toHex(gcmTag), gcmExpectedTag);
    } finally {
        gcmEnc.close();
    }

    const gcmDec = createDecryptor("aes-256-gcm", cbcKey, gcmIv);
    try {
        assert.strictEquals(gcmDec.setAAD(gcmAad), gcmDec);
        assert.strictEquals(gcmDec.setAuthTag(gcmTag), gcmDec);
        gcmDec.update(gcmCt);
        assert.strictEquals(toHex(gcmDec.final()), toHex(gcmPlain));
    } finally {
        gcmDec.close();
    }

    // Tampered GCM tag & missing tag & unsupported methods
    const badGcm = createDecryptor("aes-256-gcm", cbcKey, gcmIv);
    try {
        const wrongTag = gcmTag.slice();
        wrongTag[0] ^= 0xff;
        badGcm.setAAD(gcmAad);
        badGcm.setAuthTag(wrongTag);
        badGcm.update(gcmCt);
        expectCryptoError(() => badGcm.final(), "NEO_CRYPTO_AUTH");
    } finally {
        badGcm.close();
    }

    const noTag = createDecryptor("aes-256-gcm", cbcKey, gcmIv);
    try {
        noTag.update(gcmCt);
        expectCryptoError(() => noTag.final(), "NEO_CRYPTO_TAG");
    } finally {
        noTag.close();
    }

    expectCryptoError(() => createEncryptor("aes-128-cbc", cbcKey, cbcIv), "NEO_CRYPTO_ALGORITHM");
    expectCryptoError(() => createEncryptor("aes-256-cbc", new Uint8Array(16), cbcIv), "NEO_CRYPTO_KEY");
    expectCryptoError(() => createEncryptor("aes-256-cbc", cbcKey, new Uint8Array(12)), "NEO_CRYPTO_IV");
    expectCryptoError(() => createEncryptor("aes-256-gcm", cbcKey, new Uint8Array(16)), "NEO_CRYPTO_IV");
    assert.throws(() => createEncryptor("aes-256-gcm", cbcKey, gcmIv, { padding: false }), TypeError);
    assert.throws(() => createEncryptor("aes-256-cbc", cbcKey, cbcIv, { unknown: true }), TypeError);

    const CipherCtor = Object.getPrototypeOf(enc).constructor;
    assert.strictEquals(CipherCtor.name, "Cipher");
    assert.throws(() => new CipherCtor(), TypeError);
});

test("createHash and createHmac support sha256/sha384/sha512 and all encodings", () => {
    const sha256Abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const sha384Abc =
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7";
    const sha512Abc =
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";

    assert.strictEquals(createHash("sha256").update("abc").digest("hex"), sha256Abc);
    assert.strictEquals(createHash("SHA384").update("abc").digest("hex"), sha384Abc);
    assert.strictEquals(createHash("sha512").update("abc").digest("hex"), sha512Abc);

    // Incremental + encodings
    const h = createHash("sha256");
    assert.strictEquals(h.update("a").update("bc", "utf8"), h);
    assert.throws(() => h.digest("bogus"), TypeError); // does not consume state
    assert.strictEquals(h.digest("base64"), "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");

    assert.strictEquals(
        createHash("sha256").update("abc").digest("base64url"),
        "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0",
    );

    const raw = createHash("sha256").update("abc").digest();
    const latin1 = createHash("sha256").update("abc").digest("latin1");
    const binary = createHash("sha256").update("abc").digest("binary");
    assert.ok(raw instanceof Uint8Array && raw.length === 32);
    assert.strictEquals(latin1.length, 32);
    assert.strictEquals(binary, latin1);
    for (let i = 0; i < raw.length; i += 1) {
        assert.strictEquals(latin1.charCodeAt(i), raw[i]);
    }

    // UTF-8 lone surrogate replacement parity
    assert.strictEquals(
        createHash("sha256").update("\ud800").digest("hex"),
        createHash("sha256").update(Uint8Array.from([0xef, 0xbf, 0xbd])).digest("hex"),
    );

    // RFC 4231 HMAC-SHA256 vector
    assert.strictEquals(
        createHmac("sha256", "Jefe").update("what do ya want for nothing?").digest("hex"),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    );
    assert.strictEquals(
        createHmac("sha256", Uint8Array.from([0x4a, 0x65, 0x66, 0x65]))
            .update(Uint8Array.from([0x77, 0x68, 0x61, 0x74]))
            .digest("hex"),
        createHmac("sha256", "Jefe").update("what").digest("hex"),
    );

    // Lifecycle & errors
    expectCryptoError(() => createHash("sha1"), "NEO_CRYPTO_ALGORITHM");
    expectCryptoError(() => createHmac("md5", "k"), "NEO_CRYPTO_ALGORITHM");
    expectCryptoError(() => h.digest("hex"), "NEO_CRYPTO_DIGESTED");
    expectCryptoError(() => h.update("x"), "NEO_CRYPTO_DIGESTED");

    const closable = createHash("sha256");
    closable.close();
    closable.dispose();
    expectCryptoError(() => closable.update("a"), "NEO_CRYPTO_CLOSED");
    expectCryptoError(() => closable.digest("hex"), "NEO_CRYPTO_CLOSED");

    const HashCtor = Object.getPrototypeOf(closable).constructor;
    assert.strictEquals(HashCtor.name, "Hash");
    assert.throws(() => new HashCtor(), TypeError);
    assert.throws(() => createHash("sha256").update(new Uint8Array([1]), "utf8"), TypeError);
});

test("pbkdf2, bcryptHash, and bcryptVerify match vectors and validate bounds", () => {
    assert.strictEquals(
        toHex(pbkdf2("sha256", "passwd", "salt", 1, 64)),
        "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
    );
    assert.strictEquals(
        toHex(
            pbkdf2(
                "sha256",
                Uint8Array.from([0, 255, 112, 97, 115, 115]),
                Uint8Array.from([0, 115, 97, 108, 116, 255]),
                2,
                32,
            ),
        ),
        "60d38d8523b876fa767fb807eec4c919d8c75512bb89ad7954bc0328812eb4cc",
    );

    expectCryptoError(() => pbkdf2("sha1", "p", "s", 1, 16), "NEO_CRYPTO_ALGORITHM");
    assert.throws(() => pbkdf2("sha256", "p", "s", 0, 16), RangeError);
    assert.throws(() => pbkdf2("sha256", "p", "s", 1000001, 16), RangeError);
    assert.throws(() => pbkdf2("sha256", "p", "s", 1, 0), RangeError);
    assert.throws(() => pbkdf2("sha256", "p", "s", 1, 1025), RangeError);

    const hash = bcryptHash("pässwörd\0", 4);
    assert.ok(hash.startsWith("$2b$04$") && hash.length === 60);
    assert.strictEquals(bcryptVerify("pässwörd\0", hash), true);
    assert.strictEquals(bcryptVerify("wrong", hash), false);
    assert.strictEquals(bcryptVerify(new Uint8Array(73).fill(97), hash), false);
    assert.strictEquals(
        bcryptVerify("password", "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W"),
        true,
    );
    assert.strictEquals(
        bcryptVerify("allmine", "$2x$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"),
        false,
    );

    expectCryptoError(() => bcryptHash("a".repeat(73), 4), "NEO_CRYPTO_LIMIT");
    assert.throws(() => bcryptHash("pw", 3), RangeError);
    assert.throws(() => bcryptHash("pw", 17), RangeError);
    assert.throws(() => bcryptVerify("pw", 123), TypeError);
});

test("shamirSplit and shamirCombine split, reconstruct, and enforce validation", () => {
    const plab = shamirCombine([
        { index: 1, value: Uint8Array.of(0x51) },
        { index: 2, value: Uint8Array.of(0x64) },
    ]);
    assert.equals(Array.from(plab), [0x42]);

    const secret = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
    const shares = shamirSplit(secret, 3, 5);
    assert.strictEquals(shares.length, 5);
    for (let i = 0; i < shares.length; i += 1) {
        assert.strictEquals(shares[i].index, i + 1);
        assert.ok(shares[i].value instanceof Uint8Array);
        assert.strictEquals(shares[i].value.length, secret.length);
        assert.strictEquals(Object.isFrozen(shares[i]), true);
    }

    assert.equals(Array.from(shamirCombine([shares[0], shares[2], shares[4]], 3)), Array.from(secret));
    assert.throws(() => shamirCombine([shares[0], shares[1]], 3), RangeError);
    assert.throws(() => shamirSplit(new Uint8Array(0), 2, 3), RangeError);
    assert.throws(() => shamirSplit(secret, 3, 2), RangeError);

    expectCryptoError(
        () => shamirCombine([shares[0], { index: 1, value: shares[1].value }]),
        "NEO_CRYPTO_SHAMIR_DUPLICATE",
    );
    expectCryptoError(
        () => shamirCombine([shares[0], { index: 2, value: Uint8Array.of(1) }]),
        "NEO_CRYPTO_SHAMIR_LENGTH",
    );
});

test("Uint8Array RAB views, detached buffers, and hostile prototypes are handled safely", () => {
    const key = generateEnvelopeKey();
    const rab = new ArrayBuffer(4, { maxByteLength: 16 });
    const tracked = new Uint8Array(rab);
    tracked.set([10, 20, 30, 40]);

    // Grown tracking view
    rab.resize(8);
    tracked.set([50, 60, 70, 80], 4);
    const envGrow = encrypt(key, tracked);
    assert.equals(Array.from(decrypt(key, envGrow)), [10, 20, 30, 40, 50, 60, 70, 80]);
    assert.strictEquals(
        createHash("sha256").update(tracked).digest("hex"),
        createHash("sha256").update(Uint8Array.from([10, 20, 30, 40, 50, 60, 70, 80])).digest("hex"),
    );

    // Shrunk tracking view
    rab.resize(2);
    const envShrink = encrypt(key, tracked);
    assert.equals(Array.from(decrypt(key, envShrink)), [10, 20]);

    // Valid empty view at exact end offset
    const validEmpty = new Uint8Array(rab, 2);
    const envEmpty = encrypt(key, validEmpty);
    assert.equals(Array.from(decrypt(key, envEmpty)), []);

    // OOB RAB view on QuickJS rejects cleanly with NEO_CRYPTO_DETACHED
    if (Neo.engine === "neo-lite") {
        rab.resize(8);
        const oob = new Uint8Array(rab, 4, 4);
        rab.resize(2);
        expectCryptoError(() => encrypt(key, oob), "NEO_CRYPTO_DETACHED");
    }

    // Detached Uint8Array throws NEO_CRYPTO_DETACHED across operations
    const ab = new ArrayBuffer(8);
    const detached = new Uint8Array(ab);
    ab.transfer();
    expectCryptoError(() => encrypt(key, detached), "NEO_CRYPTO_DETACHED");
    expectCryptoError(() => decrypt(key, detached), "NEO_CRYPTO_DETACHED");
    expectCryptoError(() => createHash("sha256").update(detached), "NEO_CRYPTO_DETACHED");
    expectCryptoError(() => pbkdf2("sha256", detached, "s", 1, 16), "NEO_CRYPTO_DETACHED");
    expectCryptoError(() => bcryptHash(detached, 4), "NEO_CRYPTO_DETACHED");
    expectCryptoError(() => shamirSplit(detached, 2, 3), "NEO_CRYPTO_DETACHED");

    // Hostile Error.prototype.code & Object.prototype.code cannot hijack coded errors
    const origErrCode = Object.getOwnPropertyDescriptor(Error.prototype, "code");
    const origObjCode = Object.getOwnPropertyDescriptor(Object.prototype, "code");
    let traps = 0;
    try {
        Object.defineProperty(Error.prototype, "code", {
            configurable: true,
            set() {
                traps += 1;
            },
            get() {
                return "HIJACKED";
            },
        });
        Object.defineProperty(Object.prototype, "code", {
            configurable: true,
            set() {
                traps += 1;
            },
        });
        expectCryptoError(() => createHash("sha1"), "NEO_CRYPTO_ALGORITHM");
        expectCryptoError(() => encrypt(new Uint8Array(1), new Uint8Array(1)), "NEO_CRYPTO_KEY");
        assert.strictEquals(traps, 0);
    } finally {
        if (origErrCode) Object.defineProperty(Error.prototype, "code", origErrCode);
        else delete Error.prototype.code;
        if (origObjCode) Object.defineProperty(Object.prototype, "code", origObjCode);
        else delete Object.prototype.code;
    }
});
