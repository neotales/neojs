//! Integration tests for `vael:db`, driven through V8 `Runtime`.
#![cfg(feature = "vael-db")]

use std::fs;

use vaeljs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn module_identity_and_specifier_resolution() {
    run_js(
        r#"
import { DbError, open, openSqlite } from "vael:db";
import * as ns from "vael:db";

const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:db")) throw new Error("vael:db missing from list");
if (list.includes("db")) throw new Error("bare db listed");

const reg = Vael.getBuiltInModule("vael:db");
if (!reg || typeof reg !== "object") throw new Error("registry missing vael:db");
if (Vael.getBuiltInModule("vael:db") !== reg) throw new Error("registry not cached");
if (Vael.getBuiltInModule("db") !== undefined) throw new Error("bare db accepted");

const dyn = await import("vael:db");
if (dyn === reg) throw new Error("namespace must be distinct from registry");
if (dyn.open !== open || reg.open !== open) throw new Error("open identity mismatch");
if (dyn.openSqlite !== openSqlite || reg.openSqlite !== openSqlite) {
    throw new Error("openSqlite identity mismatch");
}
if (dyn.DbError !== DbError || reg.DbError !== DbError) {
    throw new Error("DbError identity mismatch");
}
if (Object.keys(ns).sort().join(",") !== "DbError,open,openSqlite") {
    throw new Error(`unexpected exports: ${Object.keys(ns).sort().join(",")}`);
}
if (import.meta.resolve("vael:db") !== "vael:db") throw new Error("resolve failed");
if (import.meta.resolve("VAEL:db") !== "vael:db") throw new Error("scheme case-fold failed");
"#,
    );
}

#[test]
fn sqlite_memory_and_file_crud_named_and_positional() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("app.sqlite");
    let db_path_json = serde_json::to_string(db_path.to_str().expect("utf8 path")).unwrap();

    run_js(&format!(
        r#"
import {{ open, openSqlite }} from "vael:db";

// 1. Default in-memory SQLite via openSqlite()
const mem = await openSqlite();
await mem.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER NOT NULL)");
const ins1 = await mem.execute(
    "INSERT INTO users (id, name, age) VALUES (@id, @name, @age)",
    {{ id: 1, name: "Ada", age: 36 }},
);
if (ins1.rowsAffected !== 1) throw new Error(`expected 1 row affected, got ${{ins1.rowsAffected}}`);

const ins2 = await mem.execute(
    "INSERT INTO users (id, name, age) VALUES (?, ?, ?)",
    [2, "Grace", 42],
);
if (ins2.rowsAffected !== 1) throw new Error(`expected 1 row affected, got ${{ins2.rowsAffected}}`);

const all = await mem.query("SELECT id, name, age FROM users ORDER BY id ASC");
if (all.length !== 2) throw new Error(`expected 2 rows, got ${{all.length}}`);
if (all[0].id !== 1 || all[0].name !== "Ada" || all[0].age !== 36) {{
    throw new Error(`unexpected row 0: ${{JSON.stringify(all[0])}}`);
}}
if (all[1].id !== 2 || all[1].name !== "Grace" || all[1].age !== 42) {{
    throw new Error(`unexpected row 1: ${{JSON.stringify(all[1])}}`);
}}

const upd = await mem.execute("UPDATE users SET age = @age WHERE name = @name", {{ name: "Ada", age: 37 }});
if (upd.rowsAffected !== 1) throw new Error("update affected mismatch");

const one = await mem.queryOne("SELECT name, age FROM users WHERE id = ?", [1]);
if (!one || one.name !== "Ada" || one.age !== 37) throw new Error("queryOne mismatch");

const none = await mem.queryOne("SELECT name FROM users WHERE id = ?", [999]);
if (none !== null) throw new Error(`expected null, got ${{JSON.stringify(none)}}`);

const del = await mem.execute("DELETE FROM users WHERE id = ?", [2]);
if (del.rowsAffected !== 1) throw new Error("delete affected mismatch");

await mem.close();

// 2. File-backed SQLite via open(configObject) and open(jsonString)
const filePath = {db_path_json};
const fileDb1 = await open({{ driver: "sqlite", database: filePath }});
await fileDb1.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)");
await fileDb1.execute("INSERT INTO notes (id, body) VALUES (@id, @body)", {{ id: 10, body: "persisted" }});
await fileDb1.close();

const fileDb2 = await open(JSON.stringify({{ driver: "sqlite", database: filePath }}));
const persisted = await fileDb2.queryOne("SELECT body FROM notes WHERE id = ?", [10]);
if (!persisted || persisted.body !== "persisted") {{
    throw new Error(`persistence failed: ${{JSON.stringify(persisted)}}`);
}}
await fileDb2.close();
"#,
    ));
}

#[test]
fn all_value_types_and_rab_uint8array_round_trip() {
    run_js(
        r#"
import { openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute(
    "CREATE TABLE items (id INTEGER PRIMARY KEY, n TEXT, flag INTEGER, big INTEGER, score REAL, txt TEXT, raw BLOB)"
);

const bigSafe = Number.MAX_SAFE_INTEGER;
const bigIntVal = 9_007_199_254_740_993n; // MAX_SAFE_INTEGER + 2 (beyond safe JS number)
const bytes = new Uint8Array([0, 1, 127, 128, 255]);

// Resizable ArrayBuffer view
const rab = new ArrayBuffer(4, { maxByteLength: 16 });
const rabView = new Uint8Array(rab, 1, 2);
rabView.set([0xde, 0xad]);
rab.resize(8);

await db.execute(
    "INSERT INTO items (id, n, flag, big, score, txt, raw) VALUES (@id, @n, @flag, @big, @score, @txt, @raw)",
    {
        id: bigSafe,
        n: null,
        flag: true,
        big: bigIntVal,
        score: 3.25,
        txt: "hello 🌍",
        raw: bytes,
    },
);

await db.execute(
    "INSERT INTO items (id, n, flag, big, score, txt, raw) VALUES (?, ?, ?, ?, ?, ?, ?)",
    [2, null, false, -9_007_199_254_740_995n, -1.5, "rab", rabView],
);

// Shrink RAB so rabView becomes out of bounds (length 0) -> binds empty blob cleanly.
rab.resize(1);
if (rabView.length !== 0) throw new Error("expected shrunk view length 0");
await db.execute(
    "INSERT INTO items (id, n, flag, big, score, txt, raw) VALUES (?, ?, ?, ?, ?, ?, ?)",
    [3, null, false, 0n, 0.5, "empty-rab", rabView],
);

const r1 = await db.queryOne("SELECT id, n, flag, big, score, txt, raw FROM items WHERE id = ?", [bigSafe]);
if (!r1) throw new Error("missing r1");
if (r1.id !== bigSafe || typeof r1.id !== "number") throw new Error(`safe int mismatch: ${r1.id}`);
if (r1.n !== null) throw new Error(`null mismatch: ${r1.n}`);
if (r1.flag !== 1) throw new Error(`bool mismatch: ${r1.flag}`);
if (r1.big !== bigIntVal || typeof r1.big !== "bigint") throw new Error(`bigint mismatch: ${r1.big}`);
if (r1.score !== 3.25) throw new Error(`float mismatch: ${r1.score}`);
if (r1.txt !== "hello 🌍") throw new Error(`text mismatch: ${r1.txt}`);
if (!(r1.raw instanceof Uint8Array) || r1.raw.length !== 5 || r1.raw[0] !== 0 || r1.raw[4] !== 255) {
    throw new Error("bytes roundtrip mismatch");
}

const r2 = await db.queryOne("SELECT big, raw FROM items WHERE id = ?", [2]);
if (!r2 || r2.big !== -9_007_199_254_740_995n) throw new Error("negative bigint mismatch");
if (!(r2.raw instanceof Uint8Array) || r2.raw.length !== 2 || r2.raw[0] !== 0xde || r2.raw[1] !== 0xad) {
    throw new Error("rab bytes mismatch");
}

const r3 = await db.queryOne("SELECT raw FROM items WHERE id = ?", [3]);
if (!r3 || !(r3.raw instanceof Uint8Array) || r3.raw.length !== 0) {
    throw new Error("shrunk rab bytes mismatch");
}

await db.close();
"#,
    );
}

#[test]
fn rejects_invalid_params_binding_mismatches_and_cardinality() {
    run_js(
        r#"
import { DbError, openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
await db.execute("INSERT INTO users (id, name) VALUES (1, 'Ada')");
await db.execute("INSERT INTO users (id, name) VALUES (2, 'Grace')");

async function expectCode(fn, expectedCodes, label) {
    let err = null;
    try {
        await fn();
    } catch (e) {
        err = e;
    }
    if (!err) throw new Error(`${label}: expected error, got success`);
    if (!(err instanceof DbError) || !(err instanceof Error)) {
        throw new Error(`${label}: expected DbError instance, got ${err}`);
    }
    const allowed = Array.isArray(expectedCodes) ? expectedCodes : [expectedCodes];
    if (!allowed.includes(err.code)) {
        throw new Error(`${label}: expected code in ${JSON.stringify(allowed)}, got ${err.code} (${err.message})`);
    }
    return err;
}

// Mixed named + positional
await expectCode(
    () => db.query("SELECT * FROM users WHERE id = ? AND name = @name", { name: "Ada" }),
    ["PARAMETER", "SQL"],
    "mixed named + positional",
);

// Missing named param
await expectCode(
    () => db.query("SELECT * FROM users WHERE id = @id", {}),
    "PARAMETER",
    "missing named param",
);

// Unused named param
await expectCode(
    () => db.query("SELECT * FROM users WHERE id = @id", { id: 1, extra: 2 }),
    "PARAMETER",
    "unused named param",
);

// Missing positional param
await expectCode(
    () => db.query("SELECT * FROM users WHERE id = ?", []),
    "PARAMETER",
    "missing positional param",
);

// Unused positional param
await expectCode(
    () => db.query("SELECT * FROM users WHERE id = ?", [1, 2]),
    "PARAMETER",
    "unused positional param",
);

// Cardinality > 1 in queryOne
await expectCode(
    () => db.queryOne("SELECT id FROM users"),
    "CARDINALITY",
    "queryOne cardinality",
);

// Invalid JS params -> INVALID_PARAM
const detached = new Uint8Array(new ArrayBuffer(4));
detached.buffer.transfer();

const badParams = [
    ["undefined in array", [undefined]],
    ["undefined in object", { id: undefined }],
    ["NaN", [NaN]],
    ["Infinity", [Infinity]],
    ["-Infinity", [-Infinity]],
    ["symbol", [Symbol("s")]],
    ["function", [() => {}]],
    ["nested object", [{ a: 1 }]],
    ["nested array", [[1]]],
    ["Date value", [new Date()]],
    ["top-level Date", new Date()],
    ["top-level primitive", 42],
    ["out-of-range positive bigint", [9223372036854775808n]],
    ["out-of-range negative bigint", [-9223372036854775809n]],
    ["detached Uint8Array", [detached]],
];

for (const [label, params] of badParams) {
    await expectCode(
        () => db.query("SELECT ? AS v", params),
        "INVALID_PARAM",
        label,
    );
}

await db.close();
"#,
    );
}

#[test]
fn transaction_commit_and_rollback_on_throw() {
    run_js(
        r#"
import { openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute("CREATE TABLE accounts (id INTEGER PRIMARY KEY, balance INTEGER NOT NULL)");
await db.execute("INSERT INTO accounts (id, balance) VALUES (1, 100)");

// Commit path
const committed = await db.transaction(async (tx) => {
    await tx.execute("UPDATE accounts SET balance = balance - @amt WHERE id = @id", { amt: 30, id: 1 });
    const row = await tx.queryOne("SELECT balance FROM accounts WHERE id = ?", [1]);
    return row.balance;
});
if (committed !== 70) throw new Error(`unexpected committed balance: ${committed}`);
const afterCommit = await db.queryOne("SELECT balance FROM accounts WHERE id = ?", [1]);
if (afterCommit.balance !== 70) throw new Error("commit not persisted");

// Rollback path with custom error preserved
class CustomBoom extends Error {
    constructor() {
        super("business rule failed");
        this.tag = "KEEP_ME";
    }
}

let leakedTx = null;
let caught = null;
try {
    await db.transaction(async (tx) => {
        leakedTx = tx;
        await tx.execute("UPDATE accounts SET balance = 999 WHERE id = 1");
        throw new CustomBoom();
    });
} catch (err) {
    caught = err;
}
if (!(caught instanceof CustomBoom) || caught.tag !== "KEEP_ME") {
    throw new Error(`original error not rethrown cleanly: ${caught}`);
}
const afterRollback = await db.queryOne("SELECT balance FROM accounts WHERE id = ?", [1]);
if (afterRollback.balance !== 70) {
    throw new Error(`rollback did not discard changes: ${afterRollback.balance}`);
}

// Leaked tx handle must refuse post-transaction use with CLOSED
let postTxErr = null;
try {
    await leakedTx.query("SELECT 1");
} catch (err) {
    postTxErr = err;
}
if (!postTxErr || postTxErr.code !== "CLOSED") {
    throw new Error(`expected CLOSED on leaked tx, got ${postTxErr}`);
}

await db.close();
"#,
    );
}

#[test]
fn close_idempotency_post_close_refusal_and_async_dispose() {
    run_js(
        r#"
import { openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute("SELECT 1");

await db.close();
// Idempotent second close
await db.close();

for (const op of [
    () => db.execute("SELECT 1"),
    () => db.query("SELECT 1"),
    () => db.queryOne("SELECT 1"),
    () => db.transaction(async () => {}),
]) {
    let err = null;
    try {
        await op();
    } catch (e) {
        err = e;
    }
    if (!err || err.code !== "CLOSED") {
        throw new Error(`expected CLOSED after close, got ${err}`);
    }
}

// await using / Symbol.asyncDispose
let disposedHandle = null;
{
    await using scoped = await openSqlite();
    disposedHandle = scoped;
    if (typeof scoped[Symbol.asyncDispose] !== "function") {
        throw new Error("missing Symbol.asyncDispose");
    }
    const row = await scoped.queryOne("SELECT 42 AS answer");
    if (!row || row.answer !== 42) throw new Error("query inside await using failed");
}

let errAfterScope = null;
try {
    await disposedHandle.query("SELECT 1");
} catch (e) {
    errAfterScope = e;
}
if (!errAfterScope || errAfterScope.code !== "CLOSED") {
    throw new Error(`expected CLOSED after await using exit, got ${errAfterScope}`);
}
"#,
    );
}

#[test]
fn unsupported_network_driver_and_provider_errors_are_redacted() {
    run_js(
        r#"
import { DbError, open, openSqlite } from "vael:db";

const secretHost = "secret-db.internal.example";
const secretUser = "super_secret_user";
const secretPass = "super_secret_password_987";
const secretDbName = "secret_prod_db";

let netErr = null;
try {
    await open({
        driver: "postgres",
        host: secretHost,
        port: 5432,
        database: secretDbName,
        user: secretUser,
        password: secretPass,
        tls: true,
    });
} catch (e) {
    netErr = e;
}
if (!(netErr instanceof DbError)) throw new Error(`expected DbError, got ${netErr}`);
if (netErr.code !== "DRIVER_UNAVAILABLE") {
    throw new Error(`expected DRIVER_UNAVAILABLE, got ${netErr.code}`);
}
for (const secret of [secretHost, secretUser, secretPass, secretDbName]) {
    if (netErr.message.includes(secret) || String(netErr).includes(secret)) {
        throw new Error(`network error leaked secret: ${secret}`);
    }
}

// Invalid config does not leak config values
let cfgErr = null;
try {
    await open({
        driver: "sqlite",
        database: "/tmp/secret-path/app.db",
        password: secretPass,
    });
} catch (e) {
    cfgErr = e;
}
if (!(cfgErr instanceof DbError) || cfgErr.code !== "CONFIG") {
    throw new Error(`expected CONFIG, got ${cfgErr}`);
}
for (const secret of ["/tmp/secret-path/app.db", secretPass]) {
    if (cfgErr.message.includes(secret)) {
        throw new Error(`config error leaked value: ${secret}`);
    }
}

// Provider constraint error does not leak SQL or param value
const db = await openSqlite();
await db.execute("CREATE TABLE uniq (id INTEGER PRIMARY KEY, secret TEXT NOT NULL)");
await db.execute("INSERT INTO uniq (id, secret) VALUES (1, ?)", ["top_secret_val"]);

let dbErr = null;
try {
    await db.execute("INSERT INTO uniq (id, secret) VALUES (1, ?)", ["leaked_param_xyz"]);
} catch (e) {
    dbErr = e;
}
if (!(dbErr instanceof DbError) || dbErr.code !== "DATABASE") {
    throw new Error(`expected DATABASE, got ${dbErr}`);
}
if (dbErr.databaseKind !== "Constraint") {
    throw new Error(`expected databaseKind === 'Constraint', got ${dbErr.databaseKind}`);
}
for (const sensitive of ["uniq", "leaked_param_xyz", "INSERT"]) {
    if (dbErr.message.includes(sensitive)) {
        throw new Error(`database error leaked sensitive text: ${sensitive}`);
    }
}
await db.close();
"#,
    );
}

#[test]
fn i64_min_max_and_proto_column_round_trip() {
    run_js(
        r#"
import { openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute("CREATE TABLE extremes (id INTEGER PRIMARY KEY, val INTEGER NOT NULL)");
const minI64 = -9223372036854775808n;
const maxI64 = 9223372036854775807n;

await db.execute("INSERT INTO extremes (id, val) VALUES (?, ?)", [1, minI64]);
await db.execute("INSERT INTO extremes (id, val) VALUES (?, ?)", [2, maxI64]);

const rMin = await db.queryOne("SELECT val FROM extremes WHERE id = ?", [1]);
if (!rMin || rMin.val !== minI64 || typeof rMin.val !== "bigint") {
    throw new Error(`i64::MIN mismatch: ${rMin?.val}`);
}

const rMax = await db.queryOne("SELECT val FROM extremes WHERE id = ?", [2]);
if (!rMax || rMax.val !== maxI64 || typeof rMax.val !== "bigint") {
    throw new Error(`i64::MAX mismatch: ${rMax?.val}`);
}

// Column literally named "__proto__" becomes an own data property without polluting prototype
const protoRow = await db.queryOne('SELECT 1 AS "__proto__"');
if (!protoRow) throw new Error("missing protoRow");
if (Object.getPrototypeOf(protoRow) !== Object.prototype) {
    throw new Error("row prototype was altered by __proto__ column");
}
const desc = Object.getOwnPropertyDescriptor(protoRow, "__proto__");
if (!desc || desc.value !== 1 || desc.enumerable !== true) {
    throw new Error(`__proto__ own data property mismatch: ${JSON.stringify(desc)}`);
}

await db.close();
"#,
    );
}

#[test]
fn hardened_param_parsing_rejects_proxies_getters_sparse_and_tampered_prototypes() {
    run_js(
        r#"
import { DbError, openSqlite } from "vael:db";

const db = await openSqlite();

async function expectInvalidParam(fn, label) {
    let err = null;
    try {
        await fn();
    } catch (e) {
        err = e;
    }
    if (!(err instanceof DbError) || err.code !== "INVALID_PARAM") {
        throw new Error(`${label}: expected INVALID_PARAM, got ${err}`);
    }
}

// 1. Sparse arrays and oversized length
const sparse1 = [, 1];
await expectInvalidParam(() => db.query("SELECT ?, ?", sparse1), "leading sparse hole");

const sparse2 = [1, , 3];
await expectInvalidParam(() => db.query("SELECT ?, ?, ?", sparse2), "middle sparse hole");

const hugeSparse = [1];
hugeSparse.length = 0x80000000;
await expectInvalidParam(() => db.query("SELECT ?", hugeSparse), "huge sparse array");

// Too many dense params (> MAX_DB_PARAMS)
const tooMany = new Array(4097).fill(1);
await expectInvalidParam(() => db.query("SELECT 1", tooMany), "over MAX_DB_PARAMS positional");

// 2. Proxy params (must not invoke traps)
let proxyTrapCalled = false;
const proxyArr = new Proxy([1], {
    get() {
        proxyTrapCalled = true;
        return 1;
    },
    getOwnPropertyDescriptor() {
        proxyTrapCalled = true;
        return { value: 1, writable: true, enumerable: true, configurable: true };
    },
});
await expectInvalidParam(() => db.query("SELECT ?", proxyArr), "proxy array");
if (proxyTrapCalled) throw new Error("proxy array trap was invoked");

const proxyObj = new Proxy({ a: 1 }, {
    ownKeys() {
        proxyTrapCalled = true;
        return ["a"];
    },
    get() {
        proxyTrapCalled = true;
        return 1;
    },
});
await expectInvalidParam(() => db.query("SELECT @a", proxyObj), "proxy object");
if (proxyTrapCalled) throw new Error("proxy object trap was invoked");

// 3. Accessor getters on array and plain object (must never invoke getter)
let getterCalled = false;
const objWithGetter = {};
Object.defineProperty(objWithGetter, "a", {
    enumerable: true,
    get() {
        getterCalled = true;
        return 1;
    },
});
await expectInvalidParam(() => db.query("SELECT @a", objWithGetter), "object getter");
if (getterCalled) throw new Error("object getter was invoked");

const arrWithGetter = [];
Object.defineProperty(arrWithGetter, "0", {
    enumerable: true,
    get() {
        getterCalled = true;
        return 1;
    },
});
await expectInvalidParam(() => db.query("SELECT ?", arrWithGetter), "array getter");
if (getterCalled) throw new Error("array getter was invoked");

// 4. Object.prototype / Array.prototype poisoning resilience
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    configurable: true,
    set() {
        throw new Error("poisoned Object.prototype.code setter");
    },
});
Object.defineProperty(Object.prototype, "name", {
    __proto__: null,
    configurable: true,
    set() {
        throw new Error("poisoned Object.prototype.name setter");
    },
});
Object.defineProperty(Object.prototype, "value", {
    __proto__: null,
    configurable: true,
    get() {
        throw new Error("poisoned Object.prototype.value getter");
    },
});
Object.defineProperty(Object.prototype, "get", {
    __proto__: null,
    configurable: true,
    get() {
        throw new Error("poisoned Object.prototype.get getter");
    },
});
Object.defineProperty(Array.prototype, "0", {
    __proto__: null,
    configurable: true,
    get() {
        throw new Error("poisoned Array.prototype[0] getter");
    },
});

try {
    // Normal positional & named queries still work under prototype poisoning
    const row1 = await db.queryOne("SELECT ? AS x", [42]);
    if (!row1 || row1.x !== 42) throw new Error("positional query failed under poisoning");

    const row2 = await db.queryOne("SELECT @y AS y", { y: "ok" });
    if (!row2 || row2.y !== "ok") throw new Error("named query failed under poisoning");

    // DbError construction & code/name remain unpoisoned
    await expectInvalidParam(() => db.query("SELECT ?", [,]), "sparse hole under Array.prototype[0] poison");
} finally {
    delete Object.prototype.code;
    delete Object.prototype.name;
    delete Object.prototype.value;
    delete Object.prototype.get;
    delete Array.prototype["0"];
}

await db.close();
"#,
    );
}

#[test]
fn transaction_rejects_nested_tx_direct_close_and_concurrent_db_close() {
    run_js(
        r#"
import { DbError, openSqlite } from "vael:db";

const db = await openSqlite();
await db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, val TEXT NOT NULL)");

// 1. tx.transaction() and tx.close() reject with SQL
await db.transaction(async (tx) => {
    let nestedErr = null;
    try {
        await tx.transaction(async () => {});
    } catch (e) {
        nestedErr = e;
    }
    if (!(nestedErr instanceof DbError) || nestedErr.code !== "SQL") {
        throw new Error(`expected SQL for nested tx, got ${nestedErr}`);
    }

    let closeErr = null;
    try {
        await tx.close();
    } catch (e) {
        closeErr = e;
    }
    if (!(closeErr instanceof DbError) || closeErr.code !== "SQL") {
        throw new Error(`expected SQL for tx.close(), got ${closeErr}`);
    }

    await tx.execute("INSERT INTO items (id, val) VALUES (1, 'before-close')");
});

// 2. db.close() called during an active transaction:
//    tx finishes committing cleanly, while db is closed afterward.
await db.transaction(async (tx) => {
    await tx.execute("INSERT INTO items (id, val) VALUES (2, 'during-close')");
    await db.close();
    await tx.execute("INSERT INTO items (id, val) VALUES (3, 'after-db-close-still-in-tx')");
});

let postCloseErr = null;
try {
    await db.query("SELECT * FROM items");
} catch (e) {
    postCloseErr = e;
}
if (!(postCloseErr instanceof DbError) || postCloseErr.code !== "CLOSED") {
    throw new Error(`expected CLOSED on db after close during tx, got ${postCloseErr}`);
}

// 3. db.close() racing concurrently with db.transaction() start
const db2 = await openSqlite();
const txPromise = db2.transaction(async (tx) => {
    await tx.execute("SELECT 1");
    return "ok";
});
const closePromise = db2.close();
const [txSettled] = await Promise.allSettled([txPromise, closePromise]);
if (txSettled.status === "fulfilled") {
    if (txSettled.value !== "ok") throw new Error(`unexpected tx result: ${txSettled.value}`);
} else if (!(txSettled.reason instanceof DbError) || txSettled.reason.code !== "CLOSED") {
    throw new Error(`unexpected tx race rejection: ${txSettled.reason}`);
}
"#,
    );
}

#[test]
fn sqlite_uri_prefixes_and_query_flags_are_rejected_in_js() {
    run_js(
        r#"
import { DbError, open, openSqlite } from "vael:db";

for (const bad of [
    "file:app.sqlite",
    "file::memory:?cache=shared",
    "file:app.sqlite?mode=ro",
    "app.sqlite?vfs=memdb",
]) {
    let err1 = null;
    try {
        await openSqlite(bad);
    } catch (e) {
        err1 = e;
    }
    if (!(err1 instanceof DbError) || err1.code !== "CONFIG") {
        throw new Error(`expected CONFIG for openSqlite(${bad}), got ${err1}`);
    }

    let err2 = null;
    try {
        await open({ driver: "sqlite", database: bad });
    } catch (e) {
        err2 = e;
    }
    if (!(err2 instanceof DbError) || err2.code !== "CONFIG") {
        throw new Error(`expected CONFIG for open(${bad}), got ${err2}`);
    }
}
"#,
    );
}
