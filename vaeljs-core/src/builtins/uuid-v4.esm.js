// The `uuid` version 4 module as seen by
// `import ... from "vael:uuid/v4"`.
//
// It is a thin module over `uuid.v4`. The named `v4` export and the default
// export are the same function object as `Vael.getBuiltInModule("vael:uuid").v4`, so
// every spelling shares identity. Generation, randomness, output, and errors
// are documented in `vaeljs-core/src/uuid.rs` and the `vael:uuid` module.
//
// A version 4 UUID is 122 random bits with the version `0100` and variant `10`.
// It carries no timestamp or sequence, so it has no order and must not be used
// as a sort key.

const v4 = Vael.getBuiltInModule("vael:uuid/v4").v4;

export { v4 };
export default v4;
