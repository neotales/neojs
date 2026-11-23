---
title: os
description: Build-target platform and architecture constants, not a probe of the running machine.
---

`os` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `os` or
`neo:os`; the registry form is `Neo.getBuiltInModule("os")`.

It exports two strings and has no default export. NeoJS installs no `os` global.

## Exports

| Export | Value |
| --- | --- |
| `platform` | Rust's `std::env::consts::OS` for the compilation target |
| `arch` | Rust's `std::env::consts::ARCH` for the compilation target |

```js
import { arch, platform } from "os";

console.log(platform); // e.g. "linux"
console.log(arch);     // e.g. "x86_64"
```

The values describe the target the runtime was compiled for. They are not a probe of the machine the
process is running on. NeoJS does not map, normalize, or validate them against a list of its own.

`platform` can be `linux`, `macos`, `windows`, `freebsd`, `netbsd`, `openbsd`, `dragonfly`,
`android`, `ios`, `illumos`, `solaris`, `fuchsia`, `haiku`, `redox`, `emscripten`, `wasi`, or
another name Rust documents for the target. `arch` can be `x86_64`, `x86`, `aarch64`, `arm`,
`riscv64`, `powerpc64`, `s390x`, `loongarch64`, `wasm32`, or another name Rust documents.

:::warning
These are **not** Node's `process.platform` and `process.arch` values. macOS is `macos`, not
`darwin`; 64-bit ARM is `aarch64`, not `arm64`. Translate the vocabulary yourself if another API
needs it.
:::

## What it does not do

This module calls no host op and reads no environment variable, `/etc/os-release` file, registry,
sysctl value, file, or process table. It adds no permission and is not a sandbox.

`version`, `codename`, `variant`, user and keyring lookup, and scheduler generation are not part of
this module.

Only Linux x86-64 is verified in this repository. Values on Windows, macOS, and other targets are the
compiler's constants, but those platforms are unverified.
