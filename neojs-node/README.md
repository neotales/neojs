# neojs-node

Opt-in Node compatibility adapter for the V8 `neojs-core` runtime. Enable the
`node-compat` feature on `neojs-core` and construct the runtime with
`Runtime::with_node_compat`.

The only module is `node:child_process`; only `spawnSync` and `execFileSync` are
implemented, with `encoding: "utf8"`. Every asynchronous form throws
`ERR_NODE_UNSUPPORTED`. The module is not in the `Neo` registry and has no bare
or `neo:` alias.

A default `Runtime::new` rejects every `node:` import. In an opted-in runtime, a
bare specifier that is a familiar Node module name (including `crypto`, `fs`,
`os`, and `path`) is rejected rather than resolved to a Neo module; use `neo:`
for Neo modules. Process execution also requires an explicit
`ChildProcessPolicy` allowlist; an empty policy imports the module but denies
every execution. There is no shell, `PATH` lookup, inherited environment, or
`stdio: inherit`, and all resources are bounded. An allowed executable still
runs with the full authority of the process; this is not a sandbox.

See `neojs-node-builtins` for the shared support matrix and limits.
