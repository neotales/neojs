// The `fetch` built-in module as seen by `import ... from "vael:fetch"`.
//
// The implementation lives in `vael.js` (spliced from `builtins/fetch.js` when
// the `vael-fetch` feature is enabled), and calls private host ops registered
// by `vaeljs-core/src/fetch.rs`. This file only re-exports the public values so
// every named export shares identity with `Vael.getBuiltInModule("vael:fetch")`.
const fetchMod = Vael.getBuiltInModule("vael:fetch");

export const FetchError = fetchMod.FetchError;
export const download = fetchMod.download;
export const downloadSync = fetchMod.downloadSync;
export const fetch = fetchMod.fetch;
export const fetchSync = fetchMod.fetchSync;
