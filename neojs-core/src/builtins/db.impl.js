(() => {
    const dbOps = Deno.core.ops;
    const opOpen = dbOps.op_neo_db_open;
    const opOpenSqlite = dbOps.op_neo_db_open_sqlite;
    const opExecute = dbOps.op_neo_db_execute;
    const opQuery = dbOps.op_neo_db_query;
    const opQueryOne = dbOps.op_neo_db_query_one;
    const opClose = dbOps.op_neo_db_close;
    const opTxBegin = dbOps.op_neo_db_tx_begin;
    const opTxExecute = dbOps.op_neo_db_tx_execute;
    const opTxQuery = dbOps.op_neo_db_tx_query;
    const opTxQueryOne = dbOps.op_neo_db_tx_query_one;
    const opTxCommit = dbOps.op_neo_db_tx_commit;
    const opTxRollback = dbOps.op_neo_db_tx_rollback;

    const dbError = Error;
    const dbTypeError = TypeError;
    const dbFreeze = Object.freeze;
    const dbDefineProperty = Object.defineProperty;
    const dbIsArray = Array.isArray;
    const dbJsonStringify = JSON.stringify;
    const dbAsyncDispose =
        typeof Symbol.asyncDispose === "symbol" ? Symbol.asyncDispose : null;

    /**
     * Sanitized database error with a stable screaming-snake-case `code`.
     */
    class DbError extends dbError {
        constructor(code, message, databaseKind) {
            super(message);
            dbDefineProperty(this, "name", {
                __proto__: null,
                value: "DbError",
                writable: true,
                enumerable: false,
                configurable: true,
            });
            dbDefineProperty(this, "code", {
                __proto__: null,
                value: code,
                writable: true,
                enumerable: true,
                configurable: true,
            });
            if (typeof databaseKind === "string") {
                dbDefineProperty(this, "databaseKind", {
                    __proto__: null,
                    value: databaseKind,
                    writable: true,
                    enumerable: true,
                    configurable: true,
                });
            }
        }
    }

    function unwrap(outcome) {
        if (outcome.ok) return outcome;
        throw new DbError(outcome.code, outcome.message, outcome.databaseKind);
    }

    function attachAsyncDispose(handle, closeFn) {
        if (dbAsyncDispose !== null) {
            dbDefineProperty(handle, dbAsyncDispose, {
                __proto__: null,
                value: closeFn,
                enumerable: false,
                writable: false,
                configurable: false,
            });
        }
        return dbFreeze(handle);
    }

    function makeTxDatabase(txId, isClosed) {
        function ensureOpen() {
            if (isClosed()) {
                throw new DbError("CLOSED", "database transaction is closed");
            }
        }

        async function execute(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opTxExecute(txId, sql, params)).value;
        }

        async function query(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opTxQuery(txId, sql, params)).value;
        }

        async function queryOne(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opTxQueryOne(txId, sql, params)).value;
        }

        async function transaction() {
            ensureOpen();
            throw new DbError("SQL", "nested transactions are not supported");
        }

        async function close() {
            ensureOpen();
            throw new DbError("SQL", "cannot close a transaction handle directly");
        }

        return attachAsyncDispose({
            execute,
            query,
            queryOne,
            transaction,
            close,
        }, close);
    }

    function makeDatabase(id) {
        let closed = false;

        function ensureOpen() {
            if (closed) {
                throw new DbError("CLOSED", "database handle is closed");
            }
        }

        async function execute(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opExecute(id, sql, params)).value;
        }

        async function query(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opQuery(id, sql, params)).value;
        }

        async function queryOne(sql, params) {
            ensureOpen();
            if (typeof sql !== "string") {
                throw new dbTypeError("sql must be a string");
            }
            return unwrap(await opQueryOne(id, sql, params)).value;
        }

        async function transaction(fn) {
            ensureOpen();
            if (typeof fn !== "function") {
                throw new dbTypeError("transaction callback must be a function");
            }
            const txId = unwrap(await opTxBegin(id)).id;
            let txClosed = false;
            const tx = makeTxDatabase(txId, () => txClosed);
            let result;
            try {
                result = await fn(tx);
            } catch (error) {
                txClosed = true;
                try {
                    await opTxRollback(txId);
                } catch {
                    // Preserve the original callback error.
                }
                throw error;
            }
            txClosed = true;
            unwrap(await opTxCommit(txId));
            return result;
        }

        async function close() {
            if (closed) return;
            closed = true;
            unwrap(opClose(id));
        }

        return attachAsyncDispose({
            execute,
            query,
            queryOne,
            transaction,
            close,
        }, close);
    }

    async function open(config) {
        let configJson;
        if (typeof config === "string") {
            configJson = config;
        } else if (config !== null && typeof config === "object" && !dbIsArray(config)) {
            try {
                configJson = dbJsonStringify(config);
            } catch {
                throw new DbError("CONFIG", "configuration could not be serialized to JSON");
            }
            if (typeof configJson !== "string") {
                throw new DbError("CONFIG", "configuration must be a JSON object or string");
            }
        } else {
            throw new DbError("CONFIG", "configuration must be a JSON object or string");
        }
        const outcome = unwrap(await opOpen(configJson));
        return makeDatabase(outcome.id);
    }

    async function openSqlite(database) {
        const target = database === undefined ? ":memory:" : database;
        if (typeof target !== "string") {
            throw new DbError("CONFIG", "database must be a string");
        }
        const outcome = unwrap(await opOpenSqlite(target));
        return makeDatabase(outcome.id);
    }

    return dbFreeze({
        DbError,
        open,
        openSqlite,
    });
})()
