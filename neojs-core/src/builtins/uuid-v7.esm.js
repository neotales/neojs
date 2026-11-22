// The `uuid` version 7 module as seen by `import ... from "uuid/v7"` or
// `import ... from "neo:uuid/v7"`.
//
// It is a thin module over `uuid.v7`. The named `v7` export and the default
// export are the same function object as `Neo.getBuiltInModule("uuid").v7`, so
// every spelling shares identity. Generation, randomness, timestamp, ordering,
// clock bounds, output, and errors are documented in `neojs-core/src/uuid.rs`
// and the `neo:uuid` module.
//
// A version 7 UUID embeds Unix milliseconds in its first 48 bits. Values from
// different milliseconds sort by that timestamp when the clock moves forward;
// values within one millisecond are random, a backward clock step can break the
// order, and this module promises no monotonicity.

const v7 = Neo.getBuiltInModule("uuid/v7").v7;

export { v7 };
export default v7;
