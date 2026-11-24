// The `test` built-in.
//
// The implementation lives in the runtime registry so
// `Neo.getBuiltInModule("neo:test")` and this module share one registry and one
// queue. This file only re-exports it. The test module is not Node's
// `node:test`: it implements `test(name, fn)` and a few runner helpers, and it
// makes no claim about Node's options, subtests, mocks, snapshots, hooks, or
// filters. `node:test` remains rejected until neojs has an explicit Node-module
// opt-in.
const builtin = Neo.getBuiltInModule("neo:test");

export const test = builtin.test;
export const run = builtin.run;
export const MAX_TESTS = builtin.MAX_TESTS;
export const TestFailureError = builtin.TestFailureError;
export default builtin.test;
