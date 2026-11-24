// The `fs` built-in module as seen by `import ... from "neo:fs"`.
//
// The implementations live in `neo.js`, which the runtime evaluates once per
// realm, and call private host ops registered by `neojs-core/src/fs.rs`.
// This file only re-exports them, so every named export is the same function
// object as the matching property of `Neo.getBuiltInModule("neo:fs")`. Nothing
// here reaches the private module registry: it reads the public module object
// and binds its properties.
const fs = Neo.getBuiltInModule("neo:fs");

export const readFile = fs.readFile;
export const readTextFile = fs.readTextFile;
export const writeFile = fs.writeFile;
export const writeTextFile = fs.writeTextFile;
export const writeFileAtomic = fs.writeFileAtomic;
export const exists = fs.exists;
export const stat = fs.stat;
export const lstat = fs.lstat;
export const readDir = fs.readDir;
export const mkdir = fs.mkdir;
export const remove = fs.remove;
export const rename = fs.rename;
export const copyFile = fs.copyFile;
export const realpath = fs.realpath;
