// The `db` built-in module as seen by `import ... from "vael:db"`.
//
// The implementation lives in `vael.js`, which the runtime evaluates once per
// realm when `vael-db` is enabled, and calls private host ops registered by
// `vaeljs-core/src/db.rs`. This file only re-exports the public values so
// every named export shares identity with `Vael.getBuiltInModule("vael:db")`.
const db = Vael.getBuiltInModule("vael:db");

export const DbError = db.DbError;
export const open = db.open;
export const openSqlite = db.openSqlite;
