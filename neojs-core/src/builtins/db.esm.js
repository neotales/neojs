// The `db` built-in module as seen by `import ... from "neo:db"`.
//
// The implementation lives in `neo.js`, which the runtime evaluates once per
// realm when `neo-db` is enabled, and calls private host ops registered by
// `neojs-core/src/db.rs`. This file only re-exports the public values so
// every named export shares identity with `Neo.getBuiltInModule("neo:db")`.
const db = Neo.getBuiltInModule("neo:db");

export const DbError = db.DbError;
export const open = db.open;
export const openSqlite = db.openSqlite;
