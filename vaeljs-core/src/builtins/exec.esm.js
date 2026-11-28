// The `exec` built-in module as seen by `import ... from "vael:exec"`.
//
// The implementation lives in `vael.js` (spliced from `builtins/exec.js` when
// the `vael-exec` feature is enabled), and calls private host ops registered by
// `vaeljs-core/src/exec.rs`. This file only re-exports the public values so
// every named export shares identity with `Vael.getBuiltInModule("vael:exec")`.
const execMod = Vael.getBuiltInModule("vael:exec");

export const ChildProcess = execMod.ChildProcess;
export const Command = execMod.Command;
export const CommandError = execMod.CommandError;
export const FileLock = execMod.FileLock;
export const NotFoundOnPathError = execMod.NotFoundOnPathError;
export const Output = execMod.Output;
export const cmd = execMod.cmd;
export const exec = execMod.exec;
export const execSync = execMod.execSync;
export const lockFile = execMod.lockFile;
export const lockFileSync = execMod.lockFileSync;
export const run = execMod.run;
export const runSync = execMod.runSync;
export const spawn = execMod.spawn;
export const which = execMod.which;
export const whichAll = execMod.whichAll;
export const whichAllSync = execMod.whichAllSync;
export const whichSync = execMod.whichSync;
