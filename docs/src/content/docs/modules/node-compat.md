---
title: Node Compatibility (node:*)
description: Opt-outable Node.js automation compatibility modules (11 in vjs, 10 in vjs-lite) using canonical node:* specifiers.
---

VaelJS includes an automation-focused subset of Node.js built-in modules backed by `vaeljs-node-core`. Every module requires its **canonical `node:*` specifier** — bare names such as `"fs"`, `"path"`, `"crypto"`, or `"child_process"` are never silently aliased to `node:*` or `vael:*` modules.

## Enabling & Opting Out

- **Default enabled**: Both `vjs` and `vjs-lite` include `node:*` compatibility via the `node-compat` Cargo feature.
- **Runtime opt-out**: Pass `--no-node-compat` to disable all `node:*` imports at runtime (`vjs --no-node-compat script.ts`, `vjs-lite test --no-node-compat`).
- **Compile-time opt-out**: Build with `--no-default-features` to exclude `vaeljs-node-core` completely.

Every `node:*` module provides both named exports and a matching `default` export.

## Availability Summary

`vjs` (V8) exposes **11 modules**; `vjs-lite` (QuickJS) exposes **10 modules** (`node:fs/promises`, `node:fs.promises`, and `node:child_process.execFile` are V8-only).

## `node:path`, `node:path/posix` & `node:path/win32`

Available in **both `vjs` and `vjs-lite`**.

| Module | Named Exports | Notes |
| --- | --- | --- |
| `node:path` | `sep`, `delimiter`, `join`, `resolve`, `normalize`, `relative`, `dirname`, `basename`, `extname`, `parse`, `format`, `isAbsolute`, `posix`, `win32` | Uses host dialect; `.posix` and `.win32` expose fixed dialects. |
| `node:path/posix` | Same exports (`sep: "/"`, `delimiter: ":"`) | Fixed POSIX dialect. |
| `node:path/win32` | Same exports (`sep: "\\"`, `delimiter: ";"`) | Fixed Win32 dialect. |

## `node:os`

Available in **both `vjs` and `vjs-lite`**.

| Export | Signature / Type | Description |
| --- | --- | --- |
| `platform`, `arch` | `() => string` | Node-vocabulary platform (`"linux"`, `"darwin"`, `"win32"`) and arch (`"x64"`, `"arm64"`). |
| `EOL` | `"\n" \| "\r\n"` | Host line-ending constant. |
| `homedir`, `tmpdir`, `hostname` | `() => string` | User home directory, temp directory, and host name. |
| `cpus` | `() => CpuInfo[]` | Logical CPU records (`{ model, speed, times: { user, nice, sys, idle, irq } }`). |
| `endianness` | `() => "BE" \| "LE"` | CPU byte order. |
| `type`, `machine` | `() => string` | OS name and machine architecture string. |

## `node:process`

Available in **both `vjs` and `vjs-lite`**.

| Export | Type / Signature | Description |
| --- | --- | --- |
| `argv` | `readonly string[]` | Command-line argument vector. |
| `cwd`, `chdir` | `() => string`, `(directory: string) => void` | Get or change the current working directory. |
| `env` | `Record<string, string \| undefined>` | Process environment variables. |
| `exit`, `exitCode` | `(code?: number) => never`, `number \| undefined` | Terminate process or set exit code. |
| `platform`, `arch`, `pid` | `string`, `string`, `number` | Target platform, architecture, and process ID. |
| `version`, `versions` | `string`, `Readonly<Record<string, string>>` | Runtime version metadata. |
| `hrtime` | `(time?: [number, number]) => [number, number]` + `.bigint(): bigint` | Monotonic high-resolution timer. |

## `node:fs` & `node:fs/promises`

`node:fs` is available in **both `vjs` and `vjs-lite`** (without `promises` on `vjs-lite`); `node:fs/promises` and `node:fs.promises` are **`vjs` (V8) only**. Path arguments accept `string` or `URL`-like `{ href, protocol }` objects.

| Module | Exports |
| --- | --- |
| `node:fs` (both) | `readFileSync`, `writeFileSync`, `appendFileSync`, `existsSync`, `statSync`, `lstatSync`, `readdirSync`, `mkdirSync`, `rmSync`, `unlinkSync`, `renameSync`, `copyFileSync`, `realpathSync`, `mkdtempSync`, `constants` (`F_OK`, `R_OK`, `W_OK`, `X_OK`, `COPYFILE_EXCL`, `COPYFILE_FICLONE`, `COPYFILE_FICLONE_FORCE`) |
| `node:fs` (V8 only) | `promises` (`FsPromises` interface matching `node:fs/promises`) |
| `node:fs/promises` (V8 only) | `readFile`, `writeFile`, `appendFile`, `stat`, `lstat`, `readdir`, `mkdir`, `rm`, `unlink`, `rename`, `copyFile`, `realpath`, `mkdtemp`, `constants` |

Supported text encodings (`TextEncoding`): `"utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii"`.

## `node:child_process`

Executes subprocesses **without spawning a shell**, with concurrent bounded pipe draining and timeout reaping.

| Export | Availability | Signature / Notes |
| --- | --- | --- |
| `spawnSync` | `vjs` & `vjs-lite` | `(command, args?, options?) => SpawnSyncReturns` (`{ status, signal, stdout, stderr, error?, timedOut }`) |
| `execFileSync` | `vjs` & `vjs-lite` | `(file, args?, options?) => Uint8Array \| string` |
| `execFile` | **`vjs` V8 only** | Returns `Promise<{ stdout, stderr }>` or accepts trailing `(err, stdout, stderr)` callback. |

`ChildProcessOptions`: `{ cwd?, env?, input?, encoding?, timeout?, maxBuffer? }`.

## `node:buffer`

Available in **both `vjs` and `vjs-lite`**. Exports `Buffer` (extending `Uint8Array`):

- **Static methods**: `from(value, encodingOrOffset?, length?)`, `alloc(size, fill?, encoding?)`, `allocUnsafe(size)`, `concat(list, totalLength?)`, `isBuffer(value)`, `byteLength(value, encoding?)`.
- **Instance methods**: `toString(encoding?, start?, end?)`, `toJSON()`, `equals(otherBuffer)`.
- **Encodings**: `"utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii"`.

## `node:url`

Available in **both `vjs` and `vjs-lite`**.

| Export | Description |
| --- | --- |
| `URL` | WHATWG URL class with `URL.canParse(url, base?)` and `URL.parse(url, base?)`. |
| `URLSearchParams` | Query parameter parser/serializer (`size`, `append`, `delete`, `get`, `getAll`, `has`, `set`, `sort`, `toString`, iterators). |
| `fileURLToPath` | `(url: string \| URL) => string` — hardened `file:` URL to local path conversion. |
| `pathToFileURL` | `(path: string) => URL` — local path to `file:` URL conversion. |

## `node:crypto`

Available in **both `vjs` and `vjs-lite`**.

| Export | Signature | Description |
| --- | --- | --- |
| `randomBytes` | `(size: number) => Uint8Array` | OS CSPRNG bytes. |
| `randomUUID` | `() => string` | RFC 9562 v4 UUID string. |
| `createHash` | `(algorithm: string) => Hash` | Incremental hash (`update`, `digest`). |
| `createHmac` | `(algorithm: string, key: BinaryLike) => Hmac` | Incremental HMAC (`update`, `digest`). |
| `pbkdf2Sync` | `(password, salt, iterations, keylen, digest) => Uint8Array` | Synchronous PBKDF2 key derivation. |
| `timingSafeEqual` | `(a: ArrayBufferView, b: ArrayBufferView) => boolean` | Constant-time byte comparison. |

## Example

```ts
import { Buffer } from "node:buffer";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cwd, env } from "node:process";
import { pathToFileURL } from "node:url";

const dir = mkdtempSync(join(tmpdir(), "vael-node-"));
try {
  const file = join(dir, `${randomUUID()}.txt`);
  writeFileSync(file, Buffer.from(`cwd=${cwd()}\n`, "utf8"));
  console.log(pathToFileURL(file).href, readFileSync(file, "utf8").trim());
  const out = execFileSync("echo", ["ok"], { encoding: "utf8", env });
  console.log(out.trim());
} finally {
  rmSync(dir, { recursive: true, force: true });
}
```
