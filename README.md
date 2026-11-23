# neojs

Runs ECMAScript modules and erasable TypeScript on V8 through Deno's Rust bindings.

[![Neotales](https://github.com/neotales/brand/raw/main/assets/logo.png)](https://github.com/neotales/neojs)

<!-- badges: crates.io, docs.rs, CI. Add once the crate name and first version exist. -->

## Install

```sh
cargo add neojs-core
```

For the opt-in, embeddable QuickJS runtime:

```sh
cargo add neojs-lite-core
```

Build the `neojs` command from a checkout:

```sh
cargo install --path neojs
```

## Command line

`neojs --help` prints a short summary of the command:

```text
Usage:
  neojs <file>                    run one module
  neojs --allow-remote <file-or-url>
  neojs --no-allow-remote <file>
  neojs test <file-or-glob>...    run test files
  neojs test --allow-remote <file-or-glob>...
  neojs test --no-allow-remote <file-or-glob>...
  neojs --allow-remote test <file-or-glob>...

Options:
  --allow-remote   allow https:, gh:, and github: entry and imports, and
                   jsr: imports; off by default
  --no-allow-remote
                   force remote loading off, overriding NEOJS_ALLOW_REMOTE
  -h, --help       show this help; alone, after `test`, or after `--allow-remote`
  -V, --version    show the version; alone, or after `--allow-remote`
  --               end option parsing, for a file or pattern beginning with `-`

Environment:
  NEOJS_ALLOW_REMOTE  1 or true enables remote loading, 0 or false keeps it off
                      (case-insensitive); an explicit flag overrides a valid
                      setting, and an invalid value is rejected at startup
```

With no arguments, `neojs` exits 2 and points at `--help`. `--allow-remote` is off by default and
may appear before the module, or before or after `test`; `--no-allow-remote` forces it off even when
`NEOJS_ALLOW_REMOTE` enables it. The module sections below document each built-in;
[Limits](#limits) lists the unsupported features and the exit codes.

## What you can do ⚡

**Run a module file.** `neojs` reads the file as an ECMAScript module, evaluates it, and waits
for top-level promises.

`hello.js`:

```js
const name = "neojs";
console.log(`hello from ${name}`);
```

```sh
neojs hello.js
```

```text
hello from neojs
```

**Import from other files.** Relative paths, absolute paths, and `file:` URLs resolve from the
module that imports them. Static imports and dynamic `import()` both work.

`main.js`:

```js
import { greet } from "./greet.js";

console.log(greet("world"));
```

`greet.js`:

```js
export function greet(name) {
    return `hello, ${name}`;
}
```

```text
hello, world
```

**Run TypeScript.** `.ts` and `.mts` files run by erasing type syntax. Both the entry file and
imported modules work, statically and dynamically, and a TypeScript module can import `.js` or
`.mjs` while JavaScript imports TypeScript.

`main.ts`:

```ts
import type { Options } from "./options.ts";

const options: Options = { name: "world" };
console.log(`hello, ${options.name}` satisfies string);
```

`options.ts`:

```ts
export interface Options {
    name: string;
}
```

Annotations, interfaces, type aliases, generics, `as`, `satisfies`, non-null assertions, optional
parameters, `declare` declarations, and type-only imports are erased. A type-only import is removed
before resolution, so `import type { X } from "./types.ts"` does not load the file.

There is **no type checking**. A type error is not reported and the program runs with whatever
values it has, so `const value: number = "text"` prints `text`. Type stripping is not
transpilation either: nothing is downlevelled, and only syntax that can be erased is accepted.

Constructs that need a transform are rejected with the stripper's reason:

- `enum`, `namespace` and `module` blocks, and parameter properties such as
  `constructor(private value: number)`.
- `import value = require(...)`, `export =`, angle-bracket type assertions (`<T>value`; write
  `value as T` instead), and decorators.

A rejection names the file and, where the stripper reports one, the line and column. Invalid
TypeScript is reported with the parser's position and source excerpt. Erased text is replaced by
spaces, so a stack trace from a `.ts` file points at the original line and column. `.tsx`, `.jsx`,
`.cts`, `.d.ts`, `.d.mts`, and `.d.cts` files are not supported.

**Run a remote module, explicitly.** Remote loading is off by default. Pass `--allow-remote` to
run an `https:` entry or to let a local module import an absolute `https:` URL:

```sh
neojs --allow-remote https://example.com/main.js
```

```js
// main.js, run with `neojs --allow-remote main.js`
import { greet } from "https://example.com/greet.js";
console.log(greet("world"));
```

**Persist remote loading with `NEOJS_ALLOW_REMOTE`.** `--allow-remote` is a per-command
choice. Setting `NEOJS_ALLOW_REMOTE=1` or `NEOJS_ALLOW_REMOTE=true` makes remote loading the
default for every `neojs` run and `neojs test` in that environment; `0` and `false` keep it off.
The words are case-insensitive, and any other value, including an empty one, is a startup error
(exit 2) rather than an implicit opt-in. An explicit `--allow-remote` or `--no-allow-remote`
overrides a valid setting; an invalid value is rejected even when an explicit flag is given, so a
typo cannot hide. This is a security-relevant persistent switch: it lets every module and test file
in that environment load and run remote code with the same authority as local code. Set it
deliberately, not in a shared shell profile.

A relative import inside a remote module resolves against that module's URL, so it stays remote.
`.js`, `.mjs`, `.ts`, and `.mts` work, statically and dynamically, and TypeScript is type-stripped
as it is locally. An entry may be an `https:` URL or a `gh:`/`github:` specifier; a module import
may also be a `jsr:` specifier, which always resolves through the public JSR registry rather than
an arbitrary host (see [Limits](#limits) for `jsr:` resolution). A remote module cannot import a
`file:` URL: there is no implicit transition from remote code to the local filesystem.

**GitHub specifiers.** With remote loading enabled, a `gh:` or `github:` specifier names one file
in a GitHub repository:

```sh
neojs --allow-remote gh:owner/repo@v1.2.3/src/main.ts
neojs --allow-remote github:owner/repo@0123456789abcdef0123456789abcdef01234567/src/main.ts
```

```js
// main.js, run with `neojs --allow-remote main.js`
import { parse } from "gh:owner/repo@v1.2.3/src/parse.ts";
```

The specifier is validated and rewritten to
`https://raw.githubusercontent.com/owner/repo/<ref>/<path>`, then loaded exactly as that HTTPS
module, so the body, byte, module, request, redirect, deadline, and cache rules are unchanged, and
relative imports inside the file stay on the raw host. Both schemes produce the same URL and
therefore the same module identity and cache entry. The reference is one segment: a tag, a branch,
or a commit SHA. A branch name containing `/` cannot be expressed because the first `/` ends the
reference; use a tag or a full commit SHA. The legacy `gh:owner/repo/path/@ref` spelling remains
accepted. Empty segments, `.` and `..` segments, `%` escapes, backslashes, queries, fragments,
whitespace, and control characters are rejected, as are owner, repository, and reference strings
outside GitHub's safe character set. A `gh:` entry or import without `--allow-remote` (or
`NEOJS_ALLOW_REMOTE`) fails with the same disabled message as an HTTPS one. GitHub is not an
allowlist and not a sandbox: a `gh:` specifier is one way to name an HTTPS URL, not a restriction on
which hosts may be fetched.

When a redirect is followed, the final URL is the module's identity and the requested URL is an
alias for it, as in a browser. `import.meta.url` is the final URL, relative imports resolve against
it, and a later import of either spelling reaches the same module instance. Two URLs that differ
only by query are two module instances. Each redirect is
checked: the target must be credential-free HTTPS and within the redirect limit, so a downgrade to
HTTP fails before the target is requested. Cross-origin HTTPS redirects are allowed because neojs
sends no cookie, authorization header, or referrer, and reads no proxy settings from the
environment. A URL with a username or password is rejected rather than forwarded.

Downloaded modules are cached automatically in one user-profile package cache:
`$XDG_CACHE_HOME/neojs/packages`, or `$HOME/.cache/neojs/packages` when `XDG_CACHE_HOME` is unset.
The cache key is a hash of the canonical URL including its query, so secrets in a query do not
appear as file names, and two inputs that differ by query are separate entries. Entries are written
atomically, so concurrent processes cannot publish a half-written module, and each entry records and
verifies its body length and SHA-256 digest. A fresh `Cache-Control: max-age` response is reused
without a request; freshness includes the upstream `Age` and `Date`, so a response that was already
old when it arrived is stale immediately. A stale entry is revalidated with
`If-None-Match`/`If-Modified-Since`, and a `304` reuses the cached body. `no-store` does not store the
body; it leaves an absent-entry tombstone so an older `200` cannot resurrect it. A response without a
freshness lifetime is revalidated on the next run. Publications are serialized per key and use
persistent monotonic revision files: a `200` loses to a newer entry or `no-store` tombstone, and a
`304` uses its own request revision rather than a later completion token, so it cannot outrank a
later-started `200` or `no-store`. Representation identity combines the validator, when present, with
the actual body digest, so a weak ETag shared by different bytes is not treated as the same
representation. It is tracked separately from the publication revision, so a newer `304` for the
same bytes can supersede an older `304` without letting an older representation win. A newer `304`
merges only headers it actually supplied, so omitting Cache-Control cannot undo an earlier
`no-cache`. A newer `304` merges only fields it supplied and, without a usable `Date`/`Age`, derives
age from the current cached metadata under the publication lock, so it cannot overwrite a more
conservative age applied by an earlier `304`. Ordering does
not depend on the wall clock. Lock waits are capped at 250 ms and the remaining fetch deadline; with a
longer deadline a held or stale lock skips the cache operation and the fetch continues to the
network, while a deadline at or below the cap may be consumed and the fetch times out before
transport. Header and metadata lines are
bounded, one file handle is used per read, the body length is checked before allocation, and
appended bytes are rejected as a cache miss. There is no signature or content-addressed store, so the cache is not integrity
protection against someone who can write it and does not pin a version;
`RemoteOptions::with_cache_dir` and `without_cache` control the directory.

A remote module's format follows its final URL after redirects, so `.js` to `.ts` is type-stripped
and `.ts` to `.js` is run as JavaScript. The runtime also enforces a per-module body limit, an
aggregate byte limit, and a remote module count limit. A module and its bytes are admitted before the
network or cache is consulted, a body is read only up to the remaining aggregate allowance, and a
rejected module is neither fetched nor cached. Each distinct requested URL consumes a module slot
even when its fetch fails, so repeatedly caught oversized imports cannot grow the graph without
bound. Retrying the same URL reuses its slot but repeats the I/O. A separate request limit counts
every network request, including retries and each redirect hop, so repeating one failed URL cannot
bypass the network bound. A failed entry preflight refunds its module, byte, and redaction charges
but not the request it already spent; create a new `Runtime` for a fresh request budget. Idle
connection pooling is disabled, so ureq cannot hide a retry of a request whose reused connection was
closed. The redaction URL set is capped by the module limit. The
deadline is per module, not a wall-clock bound on the whole program: running code can keep importing
modules until the aggregate limits are reached. These are resource bounds, not a sandbox
or a CPU or V8 heap bound.

Generated V8 errors redact known remote module URLs from stack-frame locations and only from
messages whose error name and prefix match a generated diagnostic shape. The module's identity and
`import.meta.url` stay the final URL. A script-supplied exception message, including a quoted
relative URL-like string, text shaped like "Cannot find module", or ` at <URL>:`, and a custom stack
are preserved, so code can still expose a URL it puts in its own error. A user-thrown
`SyntaxError`/`TypeError` that exactly copies a generated prefix and location shape is
indistinguishable from V8's own text and may be redacted. Do not use module specifiers as secret
storage.

Remote loading is a capability switch, not a permission system. It is **not SSRF protection**: there
is no host allowlist, private-address rejection, or DNS pinning, and neojs fetches whatever HTTPS URL
the program names with the process's network authority. Remote code runs with the same authority as
local code and is not sandboxed. The body limit, redirect limit, and deadline are resource bounds;
see [`RemoteOptions`](neojs-core/src/remote.rs).

**Send logs to the right stream.** `console.log`, `console.info`, and `console.debug` write to
stdout. `console.warn` and `console.error` write to stderr.

```js
console.log("out");
console.error("err");
```

**Use the console module.** The `console` built-in exports the console object the runtime installed
in the realm. A module can import it instead of relying on the global, and `neo:console` is the
canonical and only module specifier. `Neo.getBuiltInModule("neo:console")` returns that object,
and the default export's value is that same object. It is the object `globalThis.console` named
before user code can replace the whole global property.

```js
import console from "neo:console";
import { warn } from "neo:console";

console.log("stdout");
warn("stderr");
```

The default export's value is the console object. The named exports are its five neojs-defined
methods: `log`, `debug`, and `info` write to stdout, and `warn` and `error` write to stderr. The
runtime installs those five function objects before any module runs. In a realm where nothing
replaced them, the named exports are those installed functions. They are read from the console
object when the console module is evaluated, so a method replaced before the first import is what the
named export captures. Importing the module itself changes no output behaviour.

Identity and mutation semantics:

- The runtime installs `log`, `debug`, `info`, `warn`, and `error` on the global console before any
  module runs. Those are the methods a fresh realm has, and the function objects the module exposes
  when no earlier module replaced them.
- The registry object and the default export's value are the console object the runtime installed,
  not a frozen copy. Replacing a method through the default export, the registry, or
  `globalThis.console` while they still name that object changes all three, because they are the
  same object. Code that replaces a console method continues to work as it did before the module
  existed.
- Replacing the whole `globalThis.console` property does not change the registry object or the
  default export's value; they keep the console object the runtime installed. The ESM module reads
  the registry, not `globalThis.console`. Replace a method on the installed object instead.
- The named exports are read and captured when the console module is evaluated. A method replaced
  before that first evaluation is what the named export captures. Replacing a method later does not
  change the named binding, which keeps the function object it captured. Read the property from the
  default export's value or the registry to follow later replacements.
- The ES module namespace object (`import * as ns`) is a separate object from the registry object,
  and is not the console object. Its `default` property is the registry object, and its named
  properties are the captured function objects. Assigning to a namespace property throws
  `TypeError`; the console object itself is not frozen, so assigning through the default export or
  the registry is allowed and changes the global console.
- Adding a property to a shared function object is visible through every reference to that function,
  because the references are the same object.
- The console object also carries whatever other properties the engine's global console has. Those
  are not part of neojs's documented surface. None of this is a permission, sandbox, or isolation
  boundary: the console module writes to the same process streams as the global console and adds no
  new capability.

**Inspect the runtime.** `Neo.engine` names the engine that installed the global. This V8 runtime
reports `"neo"`. `Neo.getBuiltInModulesList()` returns one frozen array of the canonical `neo:`
specifiers of every implemented built-in module, in registration order. The `uuid/v4` and `uuid/v7`
subpaths appear as their canonical specifiers too.

```js
Neo.engine; // "neo"
Neo.getBuiltInModulesList();
// ["neo:age", "neo:assert", "neo:chars", "neo:console",
//  "neo:cron", "neo:crypto", "neo:os", "neo:path", "neo:slices",
//  "neo:ssh", "neo:stdio", "neo:strings", "neo:test", "neo:uuid",
//  "neo:uuid/v4", "neo:uuid/v7"]
```

The list is a snapshot of the current runtime, not a promise of a future module, and it lists no
Node modules: Node support is a separate opt-in and never enters this registry. The same frozen array is returned for every call,
so a caller cannot change later results; a new `Runtime` allocates a new array. `Neo.getBuiltInModule`
takes the same canonical `neo:` specifier: `Neo.getBuiltInModule("neo:age")` returns the module, and
`Neo.getBuiltInModule("age")` returns `undefined`. Imports follow the same rule; the bare name `age`
is not a built-in module specifier. The V8 build enables the positive `neo-chars` and `neo-slices`
features by default; `--no-default-features` removes both modules, their realm-shim source, their
registry entries, and their loader entries. The `slices` implementation is a separate source file
included only when `neo-slices` is enabled, so a disabled build does not embed its bytes.
`neo-slices` implies `neo-chars` because `CharSlice` captures `chars.toUpper`/`chars.toLower`.
Features are additive across the dependency graph and are not a sandbox.

The QuickJS runtime installs the same `Neo` shape. It reports `Neo.engine` as `"neo-lite"` and,
with its default `neo-chars` feature, implements one shared built-in, `neo:chars`.
`Neo.getBuiltInModulesList()` is one frozen array of canonical specifiers (`["neo:chars"]`), and
`Neo.getBuiltInModule("neo:chars")` returns the frozen registry object whose named functions are the
same function objects the `neo:chars` namespace exports; the namespace itself is a distinct object.
With `default-features = false` the registry and list are empty. A bare `chars` name, another scheme,
or any other specifier returns `undefined`; a non-string name throws `TypeError`. That list is not a
claim that another V8 module is planned or available. The `chars` implementation and wrapper come
from the engine-neutral `neojs-builtins` crate, which both runtimes use; see
`neojs-builtins/README.md` for the shared contract and its Unicode-version limit.

**Use the stdio built-in.** `Neo.getBuiltInModule("neo:stdio")` returns a frozen module object cached
for the realm. `neo:stdio` is the canonical and only module specifier. It exports `Readable`,
`Writable`, `stdout`, `stderr`, and `MAX_BYTES`; there is no default export. It is a synchronous
byte-stream foundation, not Node's `stream` or `process` module and not the WHATWG Streams API. No
Node or Web compatibility is promised.

```js
import { Readable, Writable, stdout, stderr } from "neo:stdio";

const writer = new Writable();
writer.writeSync(Uint8Array.from([0x00, 0xff]));
writer.writeSync("é"); // 2, because the string is UTF-8
const bytes = writer.toUint8Array(); // fresh, caller-owned copy

const reader = new Readable(bytes);
const target = new Uint8Array(16);
let count;
while ((count = reader.readSync(target)) !== 0) {
    // use target.subarray(0, count); 0 is end of file
}

stderr.writeSync("diagnostic\n");
stdout.writeSync("result\n");
```

**Reading.** `new Readable(bytes)` copies the current byte range of an attached `Uint8Array` (at
most `MAX_BYTES`) into reader-owned storage. Mutating, detaching, or resizing the caller's buffer
afterwards does not change the reader. `readSync(target)` copies at most `target.byteLength` bytes
and returns the count. It can copy fewer bytes than the target holds, so loop until it returns `0`.
Exactly `0` means end of file; end of file is not an exception. An empty target throws
`NEO_STDIO_ARGUMENT` rather than returning an ambiguous zero, even at end of file. `remaining` is the
unread byte count, `closed` reports `close()`, and `close()` releases the snapshot, is idempotent,
and never detaches a caller's buffer. Reads after it throw `NEO_STDIO_CLOSED`.

**Writing.** `new Writable()` creates a memory-backed writer implemented as one coalesced, owned
buffer whose capacity is bounded by `MAX_BYTES`, rather than a chunk or record per write. An empty
write retains nothing; growing the buffer may briefly hold the old and new buffer at once.
`writeSync(chunk)` accepts a `Uint8Array` or a string encoded as UTF-8 and returns the byte count
(not UTF-16 code units). A memory writer is all-or-nothing: it appends the whole chunk or throws
without changing the stream, and it never returns a short count. `length` is the retained byte count,
and `toUint8Array()` copies only the used prefix into a fresh caller-owned array. `end()` retires the
writer but keeps memory-backed content readable; `close()` is terminal and releases it. Both are
idempotent. A closed writer reports `length = 0` and throws `NEO_STDIO_CLOSED`.

**The process streams.** `stdout` and `stderr` are one shared memory-free `Writable` each, bound to
the current process's standard output and standard error. Writing to them adds no capability beyond
the installed `console`, which already writes to those descriptors. `writeSync` blocks the isolate
until the operating system accepts every byte or an error occurs; it does not yield to the event
loop, has no timeout, and cannot be cancelled. It returns the full byte count on success. A throw can
mean that a prefix was already written, because a system write can be partially completed before
failing; there is no rollback or retry. Bytes written by this module and by `console` share the same
descriptor and are ordered by the calls that ran. `stdout.length` and `stderr.length` are always `0`
because a host writer retains nothing, and `toUint8Array()` throws `NEO_STDIO_NOT_MEMORY` (or
`NEO_STDIO_CLOSED` after `close()`). Their
`kind` is `"stdout"` or `"stderr"`. `end()` and `close()` retire only this wrapper; they do not close
the process descriptor, so `console` keeps working. Because the object is shared per realm,
retiring it through any reference is visible through every reference; `console` itself is unaffected
because it does not go through this object. There is no `stdin`: reading the process's input
would block on input and add an input capability this slice does not grant.

**Limits and errors.** `MAX_BYTES` is 8 MiB. It bounds a reader's snapshot, a single `writeSync`
chunk, and a memory writer's retained content. The limit is per stream; there is no aggregate budget
in this slice, and a `Uint8Array` returned by `toUint8Array()` is JavaScript-owned and outside it. A
refused copy or append happens before any state changes. Sizes are checked from V8's internal view
length, not a JavaScript getter. A detached view throws `NEO_STDIO_DETACHED`; a forged, proxied, or
non-view value throws `TypeError`. Offset views are copied from their own byte range; shared and
resizable backing buffers are accepted because the copy runs without executing JavaScript. Range
copies are made with V8's native view API, not `subarray`, so replacing
`Uint8Array[Symbol.species]`, `Uint8Array.prototype.constructor`, or a typed-array prototype method
cannot run JavaScript, receive the private snapshot buffer, or re-enter the module during a read.
Other failures are an ordinary `Error` with `name = "StdioError"` and a stable own `code`:
`NEO_STDIO_ARGUMENT`, `NEO_STDIO_DETACHED`, `NEO_STDIO_LIMIT`, `NEO_STDIO_CLOSED`,
`NEO_STDIO_ENDED`, `NEO_STDIO_NOT_MEMORY`, or `NEO_STDIO_WRITE`. There is no exported error
constructor; test `error.name === "StdioError"` and the `code`. Messages never include the rejected
bytes; a write failure names the stream and the operating-system error. A closed stream reports its
closed code before validating the argument. Argument counts are strict, and a callback or other
extra argument is rejected rather than ignored.

This slice is synchronous only. It returns no `Promise` and takes no callback, and it does not wrap
blocking work in an already-resolved `Promise` to look asynchronous. A future asynchronous surface
will be additive, `Promise`-based, and will perform its work on a real asynchronous path. There is
no `pipe`, `on`, event, backpressure, or `Buffer` surface, and none is promised. The module is not a
permission, sandbox, or isolation boundary; `Deno.core` remains reachable, as noted below.

**Use the path built-in.** `Neo.getBuiltInModule("neo:path")` returns a frozen module object cached
for the realm. `neo:path` is the canonical and only module specifier. It exports frozen `posix` and
`win32` namespaces plus host-selected functions, so the same code can use an explicit syntax on any
host.

```js
import { posix, win32, SEPARATOR, SEPARATOR_PATTERN } from "neo:path";

posix.join("/srv", "app", "..", "data"); // "/srv/data"
posix.parse("/srv/data/file.txt");
// { root: "/", dir: "/srv/data", base: "file.txt", ext: ".txt", name: "file" }
win32.normalize("C:/work/out/../log"); // "C:\\work\\log"
win32.resolve("C:/work/out", "../log"); // "C:\\work\\log", on any host
["app", "data"].join(SEPARATOR); // "app/data" or "app\\data"
"app///data".split(SEPARATOR_PATTERN); // ["app", "data"]
```

The module exports `join`, `normalize`, `basename`, `dirname`, `extname`, `isAbsolute`, `parse`,
`format`, `common`, `resolve`, `relative`, and `toNamespacedPath`. `SEPARATOR` and `DELIMITER` match
the selected namespace's `sep` and `delimiter`: `/` and `:` on POSIX, `\` and `;` on Windows.
`SEPARATOR_PATTERN` matches runs of host separators; on Windows it matches both `/` and `\`.

Every path argument is a primitive string; a boxed `String`, `URL`, buffer, number, `null`,
`undefined`, or other value throws `TypeError`. `basename`'s optional suffix must be a string.
`format` requires a non-null, non-array object whose `root`, `dir`, `base`, `ext`, and `name` fields
are strings or absent. `common` requires an array of strings. A NUL (U+0000) in any argument throws
`TypeError`: it cannot name a usable file, and returning a lexical result containing one would look
usable when it is not. Error messages do not repeat the rejected path.

Empty input follows the conventional dot where a path is required: `normalize("")`, `join()` with no
non-empty segment, and `dirname("")` are `"."`. `basename("")` and `extname("")` are empty,
`parse("")` has five empty fields, `isAbsolute("")` is false, and `format({})` is empty. `parse`
returns a fresh plain object on every call.

**POSIX rules.** The separator is `/` only; a backslash is an ordinary filename character.
`normalize` collapses `/` runs, removes `.`, applies `..`, preserves leading `..` only in relative
paths, and preserves at most one trailing `/`. `//` and `///` collapse to `/`; there is no distinct
`//` network root. `join` validates every argument, concatenates with `/`, and normalizes; it does
not restart at a later absolute segment, so `join("a", "/b")` is `"a/b"`. `basename` ignores
trailing separators, a separator-only path has an empty basename, and a suffix is removed only when
it is non-empty, strictly shorter than the name, and an exact suffix. `dirname` returns the text
before the final separator with trailing separators removed; a separator-only path returns `/`, and
an empty or relative path with no parent returns `"."`. `extname` ignores trailing separators; dot
files, `.`, and `..` have no extension; the last dot of the final component starts the extension.
`parse` does not normalize the whole path. `format` appends a non-empty `base` literally, even when
it is a separator, so `format({ dir: "foo", base: "/" })` is `"foo//"`.

**Windows rules.** Both `\` and `/` are input separators; `join`, `normalize`, and `format` produce
`\`. `common` is the documented exception: it compares components as written, without normalization
or case folding, and splits on `\` only, so `/` is not a component boundary there. Roots are
recognized in this order: UNC `\\server\share`; ASCII drive root `C:\`; drive-relative
`C:`; a single rooted separator; otherwise relative. Exactly two leading separators plus a non-empty
server and share form a UNC root; three or more collapse to one rooted separator. `normalize`
preserves the root and at most one trailing `\`; a bare drive becomes `C:.`; `C:..` is preserved, and
no per-drive working directory is consulted. `join("C:", "foo")` is `"C:\\foo"`; `join("//server",
"share")` is `"\\\\server\\share\\"`. A complete UNC root, `C:\`, and `C:` have an empty basename and
extension even when the share contains a dot. `dirname("C:/")` preserves the root as written.
`relative` compares roots and components case-insensitively but preserves destination spelling; a
different drive or share returns the absolute destination. `toNamespacedPath` maps drive paths to
`\\?\` and UNC paths to `\\?\UNC\`; empty, already namespaced, and device paths are returned
unchanged.

**Working directory and errors.** `resolve` works right-to-left, validates every segment, ignores
empty strings, and resets at an absolute segment. `resolve`, `relative`, and Windows
`toNamespacedPath` read the current process working directory only when their inputs do not supply a
complete base. A call takes one lazy snapshot: `relative` shares one snapshot across both endpoints
and reads cwd at most once, and does not read it at all when both endpoints are absolute. `resolve`
and Windows `toNamespacedPath` also read at most once per call. There is no runtime-local `chdir` and
no per-drive environment lookup. A failure is an ordinary `Error` with `name = "PathError"` and a
stable own `code`: `PATH_CWD` when the working
directory is unavailable or not valid UTF-8, `PATH_CWD_DIALECT` when the non-host dialect needs a
working directory, and `PATH_WINDOWS_ROOT` when Windows cannot establish a matching drive, share, or
complete namespace root. There is no `PATH_PERMISSION`; neojs has no permission system.

**Scope.** This is lexical string manipulation, not a filesystem, permission, or containment
boundary. It does not read, write, stat, or authorize a file; resolve symlinks, junctions, drive
mappings, or short names; validate Windows reserved device names or alternate data streams; fold
Unicode confusables; or promise that a result stays inside a directory. `normalize("a/../../b")` is
`"../b"`; enforce containment with the operating system after resolving the real path. Glob helpers
and file URL conversion are deliberately not implemented in this slice. This is not Node's `path`,
not `@neotales/path`, and not NeoLiteJS `neo:path`; URL inputs, coercion, and those compatibility
surfaces are not promised. The path algorithms are adapted from NeoLiteJS and `@neotales/path` under
the MIT license; see [`neojs-core/THIRD_PARTY_NOTICES.md`](neojs-core/THIRD_PARTY_NOTICES.md) for the
exact revision, adapted files, and notices.

**Use the os built-in.** `Neo.getBuiltInModule("neo:os")` returns a frozen module object that is
cached for the realm. `neo:os` is the canonical and only module specifier. The named exports are
`platform` and `arch`, and each is the same string as the matching registry property. There is no
default export, and neojs installs no `os` global.

```js
import { arch, platform } from "neo:os";

console.log(platform); // e.g. "linux"
console.log(arch);     // e.g. "x86_64"
```

The values are Rust's `std::env::consts::OS` and `std::env::consts::ARCH` for the target the runtime
was compiled for. They describe that compilation target; they are not a probe of the machine the
process is running on. The vocabulary is exactly the compiler's, and neojs does not map, normalize,
or validate it against a list of its own. `platform` can therefore be `linux`, `macos`, `windows`,
`freebsd`, `netbsd`, `openbsd`, `dragonfly`, `android`, `ios`, `illumos`, `solaris`, `fuchsia`,
`haiku`, `redox`, `emscripten`, `wasi`, or another name Rust documents for the target, and `arch`
can be `x86_64`, `x86`, `aarch64`, `arm`, `riscv64`, `powerpc64`, `s390x`, `loongarch64`, `wasm32`,
or another name Rust documents. The exact set follows the Rust version that built the runtime; this
workspace declares Rust 1.100 as its minimum. These are **not** Node's `process.platform` and
`process.arch` values: macOS is `macos`, not `darwin`, and
64-bit ARM is `aarch64`, not `arm64`. Code that needs another vocabulary must translate it itself.

This module calls no host op and reads no environment variable, `/etc/os-release` file, registry,
sysctl value, file, or process table. It adds no permission and is not a sandbox. `version`,
`codename`, `variant`, user and keyring lookup, and scheduler generation are not part of this
module. Only Linux x86-64 is verified in this repository; values on Windows, macOS, and other
targets are the compiler's constants but those platforms are unverified.

**Use the character built-in.** The global `Neo` object exposes the modules this runtime ships.
`Neo.getBuiltInModule("neo:chars")` returns a frozen module object that is cached for the realm, so
repeated lookups return the same object. An unknown name returns `undefined`; a non-string name
throws `TypeError`.

The same module is importable as an ECMAScript module. `neo:chars` is the canonical and only module
specifier. Static imports, dynamic imports, and the registry all expose the same function objects.

```js
import { isLetter, isDigit, toUpper } from "neo:chars";

console.log(isLetter("λ".codePointAt(0))); // true
console.log(isDigit("٣".codePointAt(0))); // true
console.log(toUpper("a".codePointAt(0))); // 65
```

```js
import { isLetter } from "neo:chars";

const registry = Neo.getBuiltInModule("neo:chars");
const dynamic = await import("neo:chars");

console.log(isLetter === registry.isLetter); // true
console.log(dynamic === await import("neo:chars")); // true
console.log(import.meta.resolve("neo:chars")); // neo:chars
```

Identity and mutation semantics:

- Every named export is the same function object as the matching property of
  `Neo.getBuiltInModule("neo:chars")`. They are shared, not copies, so adding a property to an exported
  function is visible through both paths. The namespace object and the registry object are different
  containers over those shared functions, not the same object.
- The module object is frozen and the module namespace bindings cannot be assigned, so neither path
  can replace a function. In module code, assigning to a namespace property throws `TypeError`.
- The module is cached per realm, so every importer in one `Runtime` sees one module namespace. A
  new `Runtime` creates a new realm and new function objects.
- `Neo.getBuiltInModule` takes the canonical `neo:` specifier: `Neo.getBuiltInModule("neo:chars")`
  returns the module, and `Neo.getBuiltInModule("chars")` returns `undefined`. The `neo:` scheme is
  matched case-insensitively, as URL schemes are, so `NEO:chars` imports the same module; the path
  `chars` is matched exactly. Leading or trailing whitespace and embedded ASCII control characters
  are rejected rather than trimmed by URL parsing. A bare `chars` import is rejected with a message
  naming `neo:chars`.
- There is no default export. `import chars from "neo:chars"` fails to link, as does importing a name
  the module does not export.

Implemented functions:

- `isChar` — a Unicode scalar value: an integer from 0 through 0x10FFFF, excluding surrogates.
- `isAscii` — a scalar value from 0 through 0x7F.
- `isLatin1` — a scalar value from 0 through 0xFF.
- `isControl` — a control character (Unicode category `Cc`).
- `isDigit` — a decimal digit (category `Nd`), not other numerals.
- `isLetter` — a letter (category `L`).
- `isLetterOrDigit` — a letter or a decimal digit.
- `isLower` — a lowercase letter (category `Ll`).
- `isUpper` — an uppercase letter (category `Lu`).
- `isPunc` — punctuation (category `P`).
- `isSpace` — whitespace: tab, line feed, vertical tab, form feed, carriage return, space, U+0085,
  and categories `Zs`, `Zl`, and `Zp`.
- `isSymbol` — a symbol (category `S`), including emoji.
- `toLower` and `toUpper` — the lowercase or uppercase code point, when the host's full mapping is
  a single code point. Expanding mappings such as `ß` to `SS` return the input unchanged.

Every predicate takes a Unicode code point as a number. It returns `false` for any value that is
not a Unicode scalar value: a non-number, a non-integer, `NaN`, a value outside 0 through 0x10FFFF,
or a surrogate (0xD800 through 0xDFFF). Predicates never throw. `toLower` and `toUpper` throw
`TypeError` for a non-number and `RangeError` for a non-scalar value. There are no string overloads
and no `...At` helpers in this slice; pass `codePointAt(0)` yourself.

Classification uses the Unicode property tables compiled into V8, so the Unicode version follows
V8. `isSpace` follows Go's `unicode.IsSpace` categories: unlike JavaScript's `\s`, it includes
U+0085 and U+00A0 but excludes U+FEFF. This module is an independent implementation informed by
`@neotales/chars` and the QuickJS `neo:chars` module. It does not port their lookup tables, and it
does not claim compatibility with either API.

**Use the strings built-in.** `Neo.getBuiltInModule("neo:strings")` returns a frozen module object
that is cached for the realm. `neo:strings` is the canonical and only module specifier. The named
exports are `equalFold`, `startsWithFold`, `endsWithFold`, `indexOfFold`, `lastIndexOfFold`, `trim`,
`trimStart`, and `trimEnd`; each is the same function object as the matching registry property, and
there is no default export. The module is plain ECMAScript: it calls no host op and adds no
capability beyond the strings it is given.

```js
import { equalFold, indexOfFold, lastIndexOfFold, trim } from "neo:strings";

console.log(equalFold("Hello WÖrLD", "hello wörld")); // true
console.log(indexOfFold("😀a", "A")); // 2 (UTF-16 code units)
console.log(lastIndexOfFold("aaa", "aa")); // 1 (overlapping)
console.log(trim("file.txt...", ".")); // "file.txt"
```

Folded matching uses ECMAScript's simple case folding, decided per code point by V8's own RegExp
`/iu` canonicalization. It is locale-independent and is not a `toLowerCase()` comparison:
`equalFold("K", "\u212A")` is `true` (Kelvin sign) and `equalFold("s", "\u017F")` is `true` (long s),
while `equalFold("I", "\u0131")` is `false` (dotless i) and `equalFold("ss", "ß")` is `false` (that
is full folding). Indices are UTF-16 code unit offsets, like `String.prototype.indexOf`, not code
point offsets; `indexOfFold` and `lastIndexOfFold` find overlapping occurrences, and an empty search
returns the clamped start index or the string length. Searching is linear in the text plus the
search string and uses O(search) extra memory: a repeated prefix cannot make it quadratic.
`lastIndexOfFold` also stops as soon as no match can begin at or before `fromIndex`, so a small limit
does not scan a long tail. A lone surrogate
is compared as itself and never matches half of a surrogate pair. Argument values are not coerced:
a non-string throws
`TypeError`, and a non-integer `fromIndex` throws `RangeError` (apart from `Infinity`, which clamps
to the length).

With no `chars`, `trim`, `trimStart`, and `trimEnd` are exactly `String.prototype.trim`,
`trimStart`, and `trimEnd`, so they use JavaScript's whitespace set, which includes U+FEFF and
excludes U+0085. This is deliberately not `neo:chars`' `isSpace`, which follows Go's
`unicode.IsSpace`. With `chars`, the argument is a set of Unicode code points, not a prefix or
suffix, and every leading or trailing code point in the set is removed: `trim("##a##", "#")` is
`"a"`, and an empty `chars` string removes nothing.

Folding and trimming are independent implementations informed by `@neotales/strings` and
`@neotales/slices`. They do not port those packages' tables and do not claim compatibility with
their code point indexing or their `Uint8Array`/`Uint16Array` buffer forms.

**Use the assert built-in.** `Neo.getBuiltInModule("neo:assert")` returns the frozen callable `assert`
function itself, cached for the realm. `neo:assert` is the canonical and only module specifier. The
default export is that function, and the named exports
`assert`, `AssertionError`, `ok`, `equals`, `notEquals`, `strictEquals`, `notStrictEquals`, `fail`,
`throws`, `doesNotThrow`, `rejects`, and `doesNotReject` are its own properties. `assert.ok` is the
callable itself. The module calls one private host op to observe a genuine native Promise; it adds
no filesystem, network, environment, or permission capability.

```js
import assert, { equals, strictEquals } from "neo:assert";

const value = 1;
assert(value, "optional message");
equals({ a: [1, 2] }, { a: [1, 2] }); // structural equality
const same = { a: 1 };
strictEquals(same, same); // Object.is identity
assert.throws(() => JSON.parse("{"), SyntaxError);
await assert.rejects(async () => {
    throw new Error("no");
});
```

`equals` is strict structural equality, not `==` and not identity. Prototypes must match. Own
enumerable string and symbol properties are compared, and getters are invoked. Arrays also compare
length and distinguish a hole from an own `undefined` property. Built-in state is compared in
addition to own properties: `Date` time values, `RegExp` source, flags, and `lastIndex`,
`ArrayBuffer`/`SharedArrayBuffer` bytes, `DataView` viewed bytes, typed-array element type, length,
and elements, boxed primitive values, and `Error` `name` and `message`. Maps compare size and match
entries without regard to insertion order; Sets likewise compare size and values without regard to
insertion order. `NaN` equals `NaN`, `-0` differs from `0`, and cycles are handled. Functions and
genuine native Promises, WeakMaps, WeakSets, and WeakRefs compare by identity. For property, array,
and typed-array differences the failure message names the first differing path, such as
`$.items[2].name`; Map and Set failures report size or entry differences and show both collections.

`strictEquals` is exactly `Object.is`: `strictEquals(NaN, NaN)` passes, `strictEquals(0, -0)` fails,
and two distinct objects are never identical. `notEquals` and `notStrictEquals` are the negations.
`equals` and `strictEquals` take `(actual, expected[, message])`; the message is optional and must be
a string. Argument counts are strict, and arguments are not coerced.

Every failed assertion throws `AssertionError`, an `Error` subclass. It has `name`
`"AssertionError"`, a stable own `code` of `"NEO_ASSERT"`, an `operator` naming the failed
assertion, and `actual` and `expected` when the assertion had them. The code is neojs-specific; it
is not Node's `ERR_ASSERTION`.

`throws(callback[, expected[, message]])` throws unless `callback` throws, and returns the thrown
value. `doesNotThrow(callback[, message])` throws if it throws, and otherwise returns its result.
`expected` is an error constructor, checked with the intrinsic `instanceof` and not a custom
`Symbol.hasInstance`, or a `RegExp` tested against the thrown error's message (or the string form of
a non-Error value), or omitted to accept any throw. A string second argument is a `TypeError`, not a
message; pass the message third. Both functions take synchronous callbacks. A callback that returns
a genuine native Promise is a `TypeError` that points at the Promise-aware form, so work that has
not run cannot be reported as a pass. The check reads the engine's internal Promise slot, not the
prototype chain, so an object that merely inherits from `Promise.prototype` is an ordinary return.

`rejects(callback[, expected[, message]])` returns a Promise that rejects with `AssertionError`
when `callback` fulfils, and otherwise records a rejection. A synchronous throw is treated as a
rejection. The callback's return value must be a genuine native Promise or a thenable. A genuine
native Promise is observed through a private host op that attaches settlement handlers without
reading an overridden `then` and without consulting `Symbol.species`. A thenable is assimilated by
reading `then` once and calling it once. For `rejects`, a throwing `then` accessor or call is
checked against `expected` as the target's rejection, so it normally passes when `expected` is
omitted or matches; for `doesNotReject`, it fails the assertion as a rejection.

For a non-thenable rejection reason, the returned Promise resolves with that reason. Because
`rejects` is an async function, a thenable or Promise reason is assimilated by native Promise
resolution: the returned Promise may resolve with that reason's fulfilment value or reject with its
rejection. The assertion has already passed when the target rejected; this later assimilation is
the return value's semantics, not a second assertion. If a reason is thenable, do not rely on the
return value to recover the original object.

`doesNotReject(callback[, message])` resolves with the fulfilment value and rejects with
`AssertionError` if the callback throws or rejects. Invalid arguments reject the returned Promise.
`assert.rejects` and `assert.doesNotReject` work with top-level await; they do not wrap blocking work
to look asynchronous.

There is deliberately no `assertEquals`, `deepEqual`, `deepStrictEqual`, `strictEqual`, `equal`, or
`notDeepEqual`. The surface is inspired by Node's `assert/strict` and Deno's `@std/assert`, but it is
not a compatibility layer for either, and it does not implement Node's `assert/strict` aliases,
`match`, `ifError`, snapshots, or a `node:assert` specifier.

Diagnostics are bounded: a formatted value is truncated at 400 characters, 50 items, and eight
levels, cycles print as `[Circular]`, and throwing getters, an unreadable prototype, or a value
that cannot be classified make the assertion fail with an `AssertionError` rather than an unrelated
exception. Formatting may invoke
getters. The comparison is recursive, so an extremely deep non-cyclic structure can exhaust the
JavaScript stack; Maps and Sets are matched in quadratic time in their size, which is appropriate
for tests but not for huge collections. `Error` `cause` and `AggregateError.errors` are
non-enumerable and are not compared. Proxies can run their traps during comparison and formatting;
this is a test helper in the caller's realm, not an isolation boundary.

**Use the slices built-in.** `Neo.getBuiltInModule("neo:slices")` returns a frozen module object that
is cached for the realm. `neo:slices` is the canonical and only module specifier. The exports are
`Slice`, `ReadOnlySlice`, `CharSlice`, `ReadOnlyCharSlice`, `slice`, `readonlySlice`, `charSlice`,
and `readonlyCharSlice`; each class or function is the same object as the matching registry
property, and there is no default export. The module is plain ECMAScript: it calls no host op, adds
no capability, and is not a sandbox or a cross-realm or cross-thread zero-copy bridge.

```js
import { slice, readonlySlice } from "neo:slices";

const base = [0, 1, 2, 3, 4];
const view = slice(base, 1, 5); // [1, 2, 3, 4] as a live view
const inner = view.slice(1, 3); // [2, 3], relative to `view`
inner.set(0, 99);
console.log(base[2]); // 99
console.log(readonlySlice(base, 1, 3).at(1)); // 99
```

A view is a live alias of an Array or a genuine typed array. Construction, `slice`, `at`, `set`,
`indexOf`, `includes`, and iteration do not copy. `copy` is the snapshot operation: it returns a new
plain Array for an Array target, or a new intrinsic typed array of the same element type for a
typed-array target, and the copy never aliases. Views do not subclass Array or typed-array classes,
and `copy` builds its result directly, so `Symbol.species` is never consulted and cannot run user
code or substitute a custom container. `indexOf` uses strict equality, so `NaN` is never found;
`includes` uses SameValueZero, so `NaN` is found.

Ranges are explicit half-open `[start, end)` bounds, defaulting to `0` and the target length. A bound
must be a number and an integer; negative, reversed, and out-of-range bounds throw `RangeError`, and
a non-number throws `TypeError`. Nothing is clamped, and negative indices are not supported. An
empty view at the end of the target is valid. The view's `length` is fixed at construction. Splicing,
truncating, or extending the target afterwards does not move or resize the view: it keeps addressing
the same absolute indices. Reading an index the target no longer has returns `undefined`; writing it
follows ordinary Array semantics and can extend the Array. A hole reads as `undefined` and becomes
an own `undefined` element in `copy`. Ordinary prototype lookup applies to holes, so an inherited
index can supply a value. Nested `slice` bounds are relative to the parent and stay aliases of the
same target; they use the parent's fixed range rather than re-reading a target that may have shrunk.
`ReadOnlySlice` has the same live reads, nested slicing, snapshot, search, and iteration without
`set`.

For typed arrays, the element type and the target's own offset and length are preserved, and every
view shares the target's buffer, so overlapping views alias each other. `Int8Array`, `Uint8Array`,
`Uint8ClampedArray`, `Int16Array`, `Uint16Array`, `Int32Array`, `Uint32Array`, `Float32Array`,
`Float64Array`, `BigInt64Array`, and `BigUint64Array` are supported, plus `Float16Array` when the V8
build provides it. BigInt element types require BigInt values; assigning a number throws the
engine's `TypeError`. A detached or shrunk typed array leaves reads `undefined` and writes ignored;
`copy` validates the fixed range and throws `RangeError` instead of inventing zeros. That check
covers an empty range: an empty view at offset 0 still fits a zero-length target, while an empty
view at a nonzero offset throws. A resizable `ArrayBuffer` that is grown back makes the fixed range
readable again. A `SharedArrayBuffer` view aliases other views of the same
buffer; this module provides no atomics and no synchronization guarantee. A `DataView`, a
`Symbol.toStringTag` lookalike, and a Proxy of a typed array are rejected with `TypeError`; the brand
comes from the engine's internal slot, not a replaceable property or `Symbol.hasInstance`. The
internal-slot getters are captured and bound before user code runs, so replacing
`Function.prototype.call` later cannot intercept a private typed-array argument. A Proxy of an Array
is an Array and is accepted, and its traps run.

A frozen Array makes writes throw `TypeError` and still reads. Accessor elements run their getter and
setter. Those behaviours are inherent in aliasing a caller's object: a view is not a snapshot and is
not an isolation boundary.

**Character buffers.** `CharSlice` is a live view over a caller-owned `Uint32Array` of Unicode code
points, and `ReadOnlyCharSlice` is its read-only form. `CharSlice.fromString(text)` allocates one
buffer, combining surrogate pairs into one code point and preserving a lone surrogate as one code
unit. `toString()` builds the result in bounded chunks, so a large buffer cannot overflow a function
argument list; it allocates the result string and chunk temporaries, not a string per code point.
`slice` allocates only a view/state object and no character buffer. The in-place
`toUpper`/`toLower` transforms allocate no new character buffer, but their simple case mapping may
allocate a temporary string per mapped code point; they avoid converting the whole buffer to a
string. The transforms apply the same simple case mapping as `neo:chars`, so `ß` stays `ß` rather
than expanding to `SS`; lone surrogates and values outside the Unicode range are left unchanged.
`set` writes one code point through to the buffer. `toArray()` returns an independent `Uint32Array`
snapshot. `ReadOnlyCharSlice` has no `set` or in-place transform: make a `CharSlice` over the same
buffer to mutate it. JS strings are immutable, so no view can alias string storage; the buffer
allocation at `fromString` and the result string at `toString` are the deliberate boundary
conversions, and in-place transforms avoid repeated whole-string conversions. A `CharSlice`
constructor does not scan its input buffer; writing an invalid value directly into the
`Uint32Array` makes `toString` throw `RangeError`. Like a typed-array view, a character view whose
buffer was detached or shrunk makes `toArray` and `toString` (and, for the mutable `CharSlice`,
`toUpper` and `toLower`) throw `RangeError` when the fixed range no longer fits, including an empty
range at a nonzero offset.

This module is informed by `@neotales/slices` but is not a port and does not claim full or
byte-for-byte parity with it. In particular, it does not copy that package's `Uint8Array`/`Uint16Array`
character conversions, which reinterpret bytes without respecting a view's offset and length, and
it uses explicit `[start, end)` bounds rather than the reference's offset/length confusion. The
reference's `ReadonlyCharSlice` spelling and some method names differ; there is no
`slices.Slice`-compatible `map`/`update`/`replace` surface in this slice. Views are aliases within
one realm, not a Rust zero-copy bridge.

**Use the cron built-in.** `Neo.getBuiltInModule("neo:cron")` returns a frozen module object cached for
the realm. The canonical `neo:cron` import and the registry expose the same `cron`, `next`, and
`CronError` values.

```js
import { cron, next } from "neo:cron";

console.log(next("*/5 * * * *", new Date("2024-01-01T00:00:00Z")).toISOString());
// 2024-01-01T00:05:00.000Z

const job = cron("0 9 * * MON-FRI", async () => {
    await doWork();
});
job.dispose();
```

`cron(expression, callback, options?)` parses a standard five-field UTC expression and returns a
frozen controller. The first tick is strictly after the call. `dispose()` is idempotent and cancels
the pending wait; when the engine provides `Symbol.dispose`, the controller also has
`[Symbol.dispose]` set to `dispose`. An undisposed job keeps the isolate alive. Disposing stops
future ticks but does not interrupt a callback that is already running, and dropping the runtime
abandons every job. There is no `ref`/`unref`, no backoff or retry option, and no name; those are
deliberately absent.

`next(expression, after?)` is a pure query: it creates no job and schedules nothing. `after` is a
`Date` or an epoch millisecond number and defaults to the host clock. The result is a `Date` for the
first occurrence strictly after `after`, or `null` when a valid expression has no occurrence in the
100-year search window, as with `0 0 31 2 *`, or when the next occurrence would fall outside the
JavaScript `Date` range of ±8.64e15 milliseconds. The returned value is therefore always a valid
`Date`, and the last representable instant has no successor. A job whose next occurrence is the
last representable minute dispatches that occurrence once and then stops and releases its host
resource, unless the preceding callback is still in flight: the normal overlap rule then skips the
final occurrence and reports `overlap`, and the job still stops and releases.

The grammar is `minute hour day-of-month month day-of-week`. Each field accepts `*`, a value, a
`start-end` range, and comma-separated lists, with an optional `/step`; a step on a bare value runs
from that value to the field maximum. Values are minute `0`-`59`, hour `0`-`23`, day-of-month
`1`-`31`, month `1`-`12` or `JAN`-`DEC`, and day-of-week `0`-`7` or `SUN`-`SAT`, where `0` and `7`
are Sunday. The three-letter and full English month and weekday names are accepted
case-insensitively. Ranges must not wrap, steps must be at least one and no greater than the field
maximum, and the extended syntax `L`, `W`, `#`, and `?` is rejected. The whole expression is limited
to 512 bytes. When both day fields are restricted, a day matches when **either** matches, the
POSIX/Vixie rule; `*/2` counts as restricted, so `0 0 */2 * MON` means every second day of the month
**or** any Monday.

There is one callback invocation at a time. A tick that arrives while the previous callback is still
running is skipped and reported as an `overlap` event. A wake-up so late that the following
occurrence is already due is reported as `missed`, and the stale occurrence is not run: the schedule
continues from the current time rather than replaying a backlog. A thrown value or rejected Promise
from the callback is reported as `error` and does not stop the schedule. Pass
`{ onEvent(event) {} }` to observe these events; unknown option keys are rejected. Overlap and
missed events are dropped when no observer is passed. With no observer, callback failures are
re-thrown from a microtask so they are not silently lost. An exception thrown by `onEvent` itself is
discarded so the schedule keeps running.

```js
const job = cron("* * * * *", async () => {
    await sync();
}, {
    onEvent(event) {
        if (event.type === "error") console.error(event.error);
    },
});
```

All times are UTC. There is no local-time interpretation, DST handling, time-zone option, seconds
field, or `@daily`-style nickname. The scheduler uses the runtime's Tokio timer rather than a
JavaScript fake-timer surface: a synchronous callback blocks the isolate and delays every timer, and
a callback that only awaits an ordinary Promise does not keep the process alive by itself. In a
`neojs test` file, dispose every job: an undisposed job is pending event-loop work and makes the
settle check fail. This is
not Deno's `Deno.cron`, Bun's `Bun.cron`, `node-cron`, or a compatibility layer for any of them.
The deterministic clock used by the tests is compiled only with the off-by-default `test-support`
feature and is absent from a normal build.

Scheduling still follows the host wall clock, including manual and NTP adjustments. A wait re-reads
the clock at least once a minute and resolves only after the clock reaches the occurrence, so a
backward adjustment postpones a tick instead of running it early; the scheduler checks once more
immediately before dispatch for the same reason. A forward adjustment is noticed within that minute,
and if it passes the following occurrence the normal `missed` rule discards the stale tick instead of
replaying a backlog. A large jump can therefore skip occurrences, and these rechecks bound the effect
of a clock change rather than making scheduling monotonic.

**Run tests.** `neojs test` takes one or more files or shell-style globs, expands them
deterministically, sorts the selected files by canonical path, removes duplicates, and runs each
file in a fresh runtime. Only `neo:test` is a module specifier; the bare name `test` is rejected.
`test(name, fn)` accepts a synchronous or async callback; `test(name)` registers a
pending test, which fails the run.

`math.test.js`:

```js
import test from "neo:test";
import assert from "neo:assert";

test("adds", () => {
    assert.equals(1 + 1, 2);
});

test("adds asynchronously", async () => {
    await Promise.resolve();
    assert.equals(2 + 2, 4);
});
```

```sh
neojs test "tests/**/*.test.js" "tests/smoke.test.js"
```

```text
test /repo/tests/math.test.js:
ok 1 - adds
ok 2 - adds asynchronously
# 2 tests, 2 passed, 0 failed
# 1 selected file(s): 2 test(s): 2 passed, 0 failed, 0 pending, 0 unhandled, 0 unsettled; 0 file(s) failed
```

Remote loading is disabled for test files too. Give `--allow-remote` before or after `test` to let
every selected file import absolute `https:` modules; `--no-allow-remote` forces it off, and
`NEOJS_ALLOW_REMOTE` selects the default for every selected file unless a flag overrides a valid
setting:

```sh
neojs --allow-remote test "tests/**/*.test.js"
neojs test --allow-remote "tests/**/*.test.js"
neojs test --no-allow-remote "tests/**/*.test.js"
```

Each file still gets a fresh runtime, so each has its own default remote request, module, byte, and
redirect limits. The user-profile package cache is shared across those fresh runtimes and with
`neojs --allow-remote <file>`, because it is keyed by URL rather than by runtime. `--` ends option
parsing, so a file literally named `--allow-remote` or `--no-allow-remote` can still be selected
after it. The remote rules are the same as for a run: only credential-free `https:` is accepted,
relative imports inside a remote module stay remote, and the cache behavior described above applies.

The runner is registration-based, not Node's `node:test`. The CLI runs it after the entry module
settles, so a test registered after a top-level `await` is included. A file that registers no tests
fails. Remote loading stays off unless `--allow-remote`, `--no-allow-remote`, or
`NEOJS_ALLOW_REMOTE` selects it, and it applies to every selected file's fresh runtime. A rejected
Promise, a rejected thenable, a returned generator object, a pending test, an
unhandled rejection, a test that does not settle within its timeout, and event-loop work that
remains after a bounded settle window are failures, so none can look like a pass. The default
timeout is 30 seconds and can be set per test:
`test("slow", fn, { timeout: 60_000 })`. `run()` returns cumulative counts and, unless called with
`{ throwOnFailure: false }`, throws `TestFailureError` (`code: "NEO_TEST_FAILURES"`) whenever that
cumulative report has a failure, including a failure from an earlier call. The host also passes
`{ settle: true }` for its bounded event-loop check; a user call inside a top-level `await` must
leave `settle` false because it would see its own pending module evaluation.

Native Promises are observed through the `assert` module's V8 Promise observation op, which uses
their real internal state rather than `then`, `constructor`, or `Symbol.species`; a subclass with
throwing prototype getters therefore fulfils, rejects, or stays pending according to what actually
happened. A non-native thenable is read once, called once, and its first settlement wins.
Nested promises and thenables are assimilated recursively, cycles fail with `NEO_TEST_CYCLE`, and a
chain longer than 1000 steps fails with `NEO_TEST_CHAIN`. Identity is checked before reading `then`
again, so a stateful getter is read once and cannot turn its own cycle into a plain value or mask it
with a later error. The per-test timeout covers the whole
chain. `NEO_TEST_OBSERVE` means the V8 observer could not inspect a native Promise, and
`NEO_TEST_NO_TIMER` means timer support was unavailable and the runner failed closed rather than
hang. A synchronous loop or a thenable that blocks or starves the isolate is still not
interruptible, and a timeout fails the test without cancelling work that is already running.

After the queue, the runner waits up to about 100 ms of event-loop turns for detached timers, async
ops, dynamic imports, and rejection events. If deno_core still reports pending work, the report is
marked unsettled and fails; an unrefed task that the event loop does not report is outside this
guarantee. Work scheduled at module top level is part of module evaluation, so a refed top-level
timer can keep `run_file` waiting before the test runner starts; keep detached work inside test
callbacks or do not schedule it. `node:` modules, including `node:test`, remain rejected by default;
the off-by-default `node-compat` opt-in adds only `node:child_process`.

A failed file does not stop later files, and each file gets its own isolate so globals, module state,
and the queue do not leak. A literal file, a malformed glob, and a glob that selects no regular file
are all errors; there is no default discovery pattern, no watch mode, no parallelism, no filtering,
no retries, no subtests, hooks, mocks, or snapshots, and no full Node compatibility claim. Test files
run with the process's full filesystem authority; this is not a sandbox. A synchronous callback that
blocks the isolate forever still hangs the process.

**Use the age built-in.** `Neo.getBuiltInModule("neo:age")` returns a frozen module object that is
cached for the realm. `neo:age` is the canonical and only module specifier. The named exports are
`generateIdentity`, `identityToRecipient`, `encrypt`, and `decrypt`, and each is the same function
object as the matching registry property. There is no default export. The module is available in
every runtime; there is no secure-context or TLS gate.

```js
import { decrypt, encrypt, generateIdentity } from "neo:age";

const pair = generateIdentity(); // { identity, recipient }
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const ciphertext = encrypt(pair.recipient, plaintext);
const returned = decrypt(pair.identity, ciphertext);

console.log(pair.identity.startsWith("AGE-SECRET-KEY-1")); // true
console.log(returned.length); // 5
```

The API is synchronous and byte-oriented: `plaintext`, `ciphertext`, and the return values are
`Uint8Array`s, and bytes are never interpreted as text. Identities and recipients are their standard
Bech32 text, `AGE-SECRET-KEY-1...` and `age1...`. The output preserves the exact bytes, including
NUL, empty input, and non-UTF-8 data.

`generateIdentity()` takes no arguments and returns a fresh `{ identity, recipient }` object. The
identity is the secret half; keep it secret. It comes from the operating system CSPRNG.
`identityToRecipient(identity)` derives the matching `age1...` recipient. Leading and trailing
whitespace is trimmed from identity and recipient text; whitespace inside the value is rejected.

`encrypt(recipient, plaintext)` returns a standard **binary** age v1 file encrypted to the X25519
recipient. `decrypt(identity, ciphertext)` accepts such a file and returns its plaintext. This slice
supports X25519 identities and recipients and binary age v1 only. It does **not** implement ASCII
armor, passphrases (scrypt), SSH keys, plugins, tagged HPKE recipients, or post-quantum recipients.
A valid age key of another kind that fits the 512-byte key-text bound is rejected with
`NEO_AGE_UNSUPPORTED`; longer key text is refused by that bound, and an armored file is rejected as
`NEO_AGE_FORMAT`. Do not treat this module as a general `age` command or a compatibility layer for
those formats.

Argument counts are strict. `generateIdentity` takes no arguments, and the other three functions
take exactly two or one as shown. Any other count, including a trailing `undefined` or a callback,
throws `TypeError` before the host op runs. A non-string identity/recipient or a non-`Uint8Array`
plaintext/ciphertext also throws `TypeError`. There is no callback form in this slice and none is
reserved: a later asynchronous API will be real, nonblocking, and `Promise`-based, and will be
additive. Nothing here wraps blocking work in an already-settled `Promise` to appear asynchronous.
A synchronous call does its work on the realm's thread, so a large payload blocks that thread until
the call returns.

Every failure other than a `TypeError` is an `Error` with a stable own `code`:

- `NEO_AGE_IDENTITY` — the identity text is not a valid age X25519 identity.
- `NEO_AGE_RECIPIENT` — the recipient text is not a valid age X25519 recipient, or its public key
  is a low-order X25519 point that cannot produce a usable shared secret.
- `NEO_AGE_UNSUPPORTED` — the key is valid age but not X25519, and fits the key-text bound.
- `NEO_AGE_FORMAT` — truncated, malformed, or unsupported file (including a passphrase file or
  ASCII armor).
- `NEO_AGE_IDENTITY_MISMATCH` — no supplied identity matched the file's recipients.
- `NEO_AGE_AUTH` — a header MAC or payload authentication tag did not verify.
- `NEO_AGE_LIMIT` — an input or output exceeded a documented bound.
- `NEO_AGE_RANDOM` — the operating system CSPRNG failed.
- `NEO_AGE_DETACHED` — a transferred `Uint8Array` view.
- `NEO_AGE_INTERNAL` — the backend reported an unexpected failure.

Messages are fixed text. They never include an identity, a recipient, plaintext, ciphertext, or key
bytes. A failed `decrypt` never returns partial or unauthenticated plaintext: the header MAC is
verified before payload decryption, every STREAM chunk is authenticated, and any failure discards the
working buffer.

Size limits are neojs limits, not age format limits. `encrypt` accepts at most 8 MiB of plaintext
and produces at most 8 MiB + 1 MiB. `decrypt` accepts at most 8 MiB + 1 MiB of ciphertext, a header
of at most 64 KiB, at most 1024 recipient stanzas, a stanza body of at most 64 KiB, and returns at
most 8 MiB of plaintext. The ciphertext and key-text bounds are checked against the view or string
length before its bytes are copied, and the decryption payload limit is enforced while authenticated
chunks are written, so the result cannot first grow past the bound. A valid external age file with a
much larger header is rejected by this policy.

A `Uint8Array` view is read through its own range, so an offset view copies only its bytes. A
detached view is `NEO_AGE_DETACHED`, not empty input. `SharedArrayBuffer`-backed and resizable
`ArrayBuffer` views are accepted; V8 exposes a view that a shrunk resizable buffer left out of
bounds as a zero-length view, which is encrypted as empty bytes. V8's internal type check is used,
so `Symbol.hasInstance`, a `byteLength` getter, or a proxy cannot change validation. The four host
ops are captured before user code runs, so replacing `Deno.core.ops` later does not change what the
module calls. None of this is isolation from other code in the same realm.

The backend is the MIT-licensed [`neotales-age`](https://github.com/neotales/rs-age) crate, a direct
age v1 implementation. It was chosen over the upstream `age` crate because its operating system
entropy failure is a returned error rather than a panic, and because it exposes explicit read limits;
the upstream crate has no public RNG injection point and panics if the CSPRNG fails, which aborts
under `panic = "abort"`. The format and X25519 handling are therefore not reimplemented here. Binary
age v1 files emitted by this module are standard: interop was checked against the `age` 1.3.2
command-line tool, and the test suite includes a fixture produced by that tool and decrypts it. An
ignored two-way test, `cargo test -p neojs --test cli -- --ignored age_cli_interop`, runs the same
roundtrip against `age` or `rage` from `PATH` (or `AGE_BIN`) and fails rather than skipping when the
tool is missing. The always-on tests cover a wrong identity, every single-bit change, every truncation
prefix, empty and binary payloads, typed-array views, module identity, the low-order-recipient error,
and the documented limits. Rejection paths are exercised too: ASCII armor and passphrase-encrypted
files are refused as `NEO_AGE_FORMAT`, and a post-quantum identity is refused as
`NEO_AGE_UNSUPPORTED` (its public key exceeds the key-text bound and is refused as
`NEO_AGE_RECIPIENT`). SSH and plugin inputs are not exercised at all; those formats are not exposed,
and this slice makes no support claim about them.

This module does not promise zeroization. The identity string, plaintext, ciphertext, and
intermediate key material are copied into Rust buffers and the JavaScript heap; those copies are
reclaimed normally but not guaranteed to be erased first. It does not store, derive, wrap, rotate,
or persist keys, and it is not a sandbox or a permission boundary. It performs no filesystem or
network access.

**Use the crypto built-in.** `Neo.getBuiltInModule("neo:crypto")` returns a frozen module object that
is cached for the realm. `neo:crypto` is the canonical and only module specifier. The named exports
are `generateEnvelopeKey`, `randomBytes`, `encrypt`, `decrypt`, `createHash`, `createHmac`,
`hashAsync`, `hmacAsync`, `pbkdf2`, `pbkdf2Async`,
`bcryptHash`, `bcryptHashAsync`, `bcryptVerify`, and `bcryptVerifyAsync`, and each is the same
function object as the matching registry property. There is no default export. The module is
available in every runtime; there is no secure-context or TLS gate.

```js
import { decrypt, encrypt, generateEnvelopeKey } from "neo:crypto";

const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
const context = Uint8Array.from([0x61, 0x2d, 0x64]);
const envelope = encrypt(key, plaintext, context);
const returned = decrypt(key, envelope, context);

console.log(returned.length); // 5
```

The API is synchronous and byte-oriented: every byte argument and every return value is a
`Uint8Array`, and bytes are never interpreted as text. A key is exactly 32 bytes.
`generateEnvelopeKey()` returns 32 bytes from the operating system CSPRNG. `associatedData` is
optional, authenticated but not encrypted, and must be supplied unchanged to `decrypt`. `undefined`
means no associated data; `null` is a `TypeError`.

`randomBytes(length)` returns a `Uint8Array` of `length` bytes from the same operating system
CSPRNG, for salts, IVs, or nonces used by another construction. `length` must be an integer from 0
through 8 MiB: a non-number throws `TypeError`, and a non-integer, negative, non-finite, or
too-large value throws `RangeError`. The call is synchronous and returns no `Promise`. There is no
`Math.random` or time-based fallback; if the operating system refuses entropy, the call throws
`NEO_CRYPTO_RANDOM` instead of returning predictable bytes. Tests cover length, type, range, and
error behaviour only; they do not and cannot prove statistical randomness or cryptographic
security.

Argument counts are strict. `generateEnvelopeKey` takes no arguments, `randomBytes` exactly one,
and `encrypt`/`decrypt` two or three. Any other count throws `TypeError` before the host call, so
`randomBytes(length, callback)` and other callback-style calls are rejected rather than silently
ignoring the extra argument. There is no callback form for `randomBytes` in this slice and none is
reserved: asynchronous APIs will be `Promise`-based and additive, not callback-based. Explicit
`undefined` in the optional associated-data position is allowed; an extra fourth argument is not.

Every `encrypt` call draws a fresh 96-bit nonce from the operating system CSPRNG. The envelope is:

| offset | bytes | field                        |
| ------ | ----- | ---------------------------- |
| 0      | 4     | magic, ASCII `NEO1`          |
| 4      | 1     | version, `1`                 |
| 5      | 1     | algorithm, `1` (AES-256-GCM) |
| 6      | 12    | nonce                        |
| 18     | n     | ciphertext                   |
| 18+n   | 16    | authentication tag           |

The first 18 bytes and the caller's associated data are authenticated as AES-GCM associated data.
Changing any envelope byte, using another key, or supplying different associated data makes
`decrypt` throw; plaintext is not returned from a failed check. An unknown magic, version, or
algorithm is reported as malformed.

A random 96-bit nonce reaches about 2^-32 collision probability at roughly 2^32 encryptions under
one key, so retire a key well before that. The caller cannot choose a nonce, which removes the
common reuse mistake, but key lifetime remains the caller's responsibility. The module does not
store, derive, wrap, rotate, or persist keys, and it does not promise zeroization: key and plaintext
bytes may remain in Rust buffers and the JavaScript heap until reclaimed. It is not a sandbox.
The crypto host ops are captured before user code runs, so replacing `Deno.core.ops` later does not
change what the module calls; this is not isolation from code in the same realm.

The module accepts at most 8 MiB of plaintext, 8 MiB of caller associated data, an envelope of
8 MiB + 34 bytes, and 8 MiB in one `randomBytes` call. These are neojs limits, not intrinsic
AES-GCM limits. Sizes are checked against the JavaScript view's byte length before its bytes are
copied. A detached view is rejected with `NEO_CRYPTO_DETACHED`. Offset views,
`SharedArrayBuffer`-backed views, and resizable `ArrayBuffer`
views are accepted; a resizable buffer that shrank until a view is out of bounds is exposed by V8
as a zero-length view and encrypted as empty bytes. A refused Rust reservation is reported as
`NEO_CRYPTO_LIMIT`, but V8, the Rust allocator, and the JavaScript heap allocate too, so recovery
from out-of-memory is not guaranteed.

An argument that is not a `Uint8Array` throws `TypeError`; a `randomBytes` length that is not a
number throws `TypeError`, and one that is not an integer or is out of range throws `RangeError`.
Other failures are `Error` objects with a stable own `code`: `NEO_CRYPTO_KEY` (key length),
`NEO_CRYPTO_FORMAT` (truncated or unknown envelope), `NEO_CRYPTO_AUTH` (wrong key, wrong
associated data, or tampering), `NEO_CRYPTO_RANDOM` (OS CSPRNG failure), `NEO_CRYPTO_LIMIT` (an
input above a documented size bound or a refused allocation), or `NEO_CRYPTO_DETACHED` (a
transferred buffer). Messages never include key, plaintext, or ciphertext bytes.

The envelope is specific to neojs. AES-256-GCM itself is standard (NIST SP 800-38D), and the
primitive is tested against published NIST/McGrew–Viega vectors. The envelope has two separately
reported checks: Rust unit tests compare the envelope path byte-for-byte with two independently
produced Node/OpenSSL fixtures by supplying each fixture's nonce to the same internal encryption
path that the public API uses; public JavaScript integration tests decrypt those fixtures and
roundtrip envelopes emitted with a fresh random nonce. Because the public API intentionally exposes
no nonce, the JavaScript layer does not perform a byte-for-byte emission comparison. Another tool
must reproduce the envelope and associated-data construction to interoperate. None of this is a
PLAB compatibility claim.

**Stateful CBC and GCM ciphers.** `createEncryptor(algorithm, key, iv[, options])` and
`createDecryptor(algorithm, key, iv[, options])` return a synchronous stateful object for
`aes-256-cbc` or `aes-256-gcm`, compared case-insensitively. The key is exactly 32 bytes. The IV is
exactly 16 bytes for CBC and 12 bytes for GCM. Other algorithm names, key length, and IV/nonce
length throw `NEO_CRYPTO_ALGORITHM`, `NEO_CRYPTO_KEY`, or `NEO_CRYPTO_IV`; values are never stretched,
truncated, or hashed into shape. The CBC primitive is RustCrypto `cbc` 0.2.1 with `aes` 0.9.4; GCM is
`aes-gcm` 0.10.3. The caller supplies the IV/nonce and is responsible for it: a GCM 12-byte nonce
must never repeat under one key, and a CBC 16-byte IV must be unpredictable and unique per key. The
module does not generate, count, or track IVs. The NEO1 `encrypt`/`decrypt` envelope draws a fresh
random nonce per call and is separate.

`update(data[, inputEncoding])` accepts a `Uint8Array` or a UTF-8 string. A string may name only
`utf8`/`utf-8`; a `Uint8Array` may not name an encoding. `update` copies into a bounded buffer and
returns an empty `Uint8Array`. `final()` performs the operation over the whole buffered message and
returns the output as a `Uint8Array`. The concatenation of all returns is the same byte stream a
streaming implementation would produce, but this slice does not release chunks early. This is
deliberate for GCM: no plaintext is returned before the tag is checked. It is a Node
`createCipheriv`-inspired surface, **not full Node compatibility**: there is no `createCipheriv` or
`createDecipheriv` export, no output encoding, no callback or Promise form, and no claim to Node's
chunk timing. The NEO1 `encrypt`/`decrypt` envelope remains a separate format and API.

CBC is confidential but **not authenticated**. This module computes no MAC and implies no MAC.
Prefer an established AEAD such as AES-256-GCM for new designs. If CBC interoperability is required,
use encrypt-then-MAC with an independent MAC key: authenticate the IV and ciphertext, verify the MAC
before decrypting, and reject the message on any MAC failure. Do not use authenticate-then-encrypt.
Beware padding oracles: exposing whether CBC padding is valid, including through distinct
`NEO_CRYPTO_PADDING` behavior, can let an attacker decrypt or forge data. PKCS#7 padding is enabled
by default and verified strictly at `final()`. Every padding byte must have the padding length; an
empty ciphertext, a non-16-byte-multiple ciphertext, or an invalid final block throws
`NEO_CRYPTO_PADDING`. `setAutoPadding(false)` or `options: { padding: false }` disables padding, and
then the plaintext or ciphertext must already be a multiple of 16 bytes.

The factory options policy is strict. `options` is either `undefined` or a non-Proxy object whose
immediate prototype is `null` or this realm's intrinsic `Object.prototype`. Validation is own-only:
every own property must be an enumerable data property with a string key. Own symbols, accessors, and
non-enumerable properties are rejected, and a custom prototype is rejected. Inherited properties on
the intrinsic `Object.prototype` or a null prototype are not read at all, so a polluted
`Object.prototype.padding` or unknown property has no effect. Arrays, class instances, functions, and
Proxies throw `TypeError` before a native resource slot is reserved. CBC permits only `padding` with
a boolean or `undefined` value; GCM permits no own properties. The value is read from the validated
own-property descriptor, not by a later `options.padding` access, and no Proxy trap or getter runs
during validation. Unknown own keys such as Node's `autoPadding` name or a misspelling such as
`paddding` are rejected rather than ignored.

GCM is authenticated. `setAAD(aad)` may be called at most once, before the first `update()` or
`final()`, and the associated data is authenticated not encrypted. A GCM encryptor's 16-byte tag is
available from `getAuthTag()` only after `final()`. A GCM decryptor must call `setAuthTag(tag)` with
exactly 16 bytes before `final()`; a missing tag throws `NEO_CRYPTO_TAG`, and a failed tag, wrong key,
wrong nonce, wrong AAD, or tampered ciphertext throws `NEO_CRYPTO_AUTH` without returning plaintext.
`final()` consumes the message; a second `final()` or a later `update()` throws `NEO_CRYPTO_STATE`.
`close()` (alias `dispose()`) releases the native state and is idempotent; a later method throws
`NEO_CRYPTO_CLOSED`.

One cipher object accepts at most 8 MiB of input and 8 MiB of GCM associated data. At most 64 cipher
objects may be open per runtime, and their combined buffered input and associated data may not exceed
64 MiB; creating past the object cap throws `NEO_CRYPTO_TOO_MANY_OBJECTS` and exceeding the aggregate
buffer cap throws `NEO_CRYPTO_BUSY`. There is no finalizer, so close objects you abandon. The module
does not promise zeroization. No asynchronous cipher API is provided: a genuine Promise form for a
stateful object would need a bounded worker design, and this slice did not add a `Promise.resolve`
wrapper that would only look asynchronous. The synchronous API works on every executor.

Example with nonsecret test vectors and mandatory disposal:

```js
import { createDecryptor, createEncryptor } from "neo:crypto";

const fromHex = (text) => Uint8Array.from(text.match(/../g), (byte) => parseInt(byte, 16));
const toHex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");

// NIST SP 800-38A F.2.5 AES-256-CBC test vector, no padding.
const cbcKey = fromHex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
const cbcIv = fromHex("000102030405060708090a0b0c0d0e0f");
const cbcPlaintext = fromHex(
    "6bc1bee22e409f96e93d7e117393172a" +
    "ae2d8a571e03ac9c9eb76fac45af8e51" +
    "30c81c46a35ce411e5fbc1191a0a52ef" +
    "f69f2445df4f9b17ad2b417be66c3710",
);

let encryptor;
try {
    encryptor = createEncryptor("aes-256-cbc", cbcKey, cbcIv, { padding: false });
    encryptor.update(cbcPlaintext);
    const ciphertext = encryptor.final();
    if (toHex(ciphertext) !==
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6" +
        "9cfc4e967edb808d679f777bc6702c7d" +
        "39f23369a9d9bacfa530e26304231461" +
        "b2eb05e2c39be9fcda6c19078c6a9d1b") {
        throw new Error("CBC vector mismatch");
    }
} finally {
    encryptor?.dispose();
}

// Nonsecret Node/OpenSSL AES-256-GCM fixture with AAD.
const gcmKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const gcmCiphertext = fromHex("6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1");
const gcmTag = fromHex("9ea6507bfbc87455b1f24edc000cc662");

function decryptGcm(tag) {
    let decryptor;
    try {
        decryptor = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
        decryptor.setAAD(gcmAad);
        decryptor.setAuthTag(tag);
        decryptor.update(gcmCiphertext);
        return decryptor.final();
    } catch (error) {
        if (error.code === "NEO_CRYPTO_AUTH") {
            throw new Error("GCM authentication failed; do not use the message");
        }
        throw error;
    } finally {
        decryptor?.dispose();
    }
}

if (toHex(decryptGcm(gcmTag)) !== "6e656f6a73206369706865722067636d20696e7465726f70") {
    throw new Error("GCM vector mismatch");
}

// A tampered tag must fail closed. The error path is part of the example so
// callers copy it; the plaintext was never returned.
const tamperedTag = gcmTag.slice();
tamperedTag[0] ^= 1;
try {
    decryptGcm(tamperedTag);
    throw new Error("tampered GCM tag was accepted");
} catch (error) {
    if (error.message !== "GCM authentication failed; do not use the message") {
        throw error;
    }
}
```

**Hash and HMAC.** `createHash(algorithm)` and `createHmac(algorithm, key)` return a synchronous
incremental object. The accepted algorithms are `sha256`, `sha384`, and `sha512`, compared
case-insensitively; other names, including `sha1`, `sha224`, and OpenSSL aliases such as `sha2-256`,
throw `NEO_CRYPTO_ALGORITHM`. `update(data[, encoding])` accepts a `Uint8Array` of exact bytes or a
string encoded as UTF-8. A string may name only `utf8`/`utf-8`; a `Uint8Array` may not name an
encoding. `update` returns the object, so calls chain. `digest()` returns a `Uint8Array`;
`digest("hex")`, `digest("base64")`, `digest("base64url")`, and `digest("latin1")` (alias `binary`)
return a string. Algorithm and encoding names are case-insensitive.

`digest` consumes the state. A second `digest`, and any `update` after a successful `digest`, throw
`NEO_CRYPTO_DIGESTED`. An unsupported digest encoding is rejected before the state is consumed, so
the object remains usable. There is no `copy()`. `close()` releases the native state without a
digest and is idempotent; `dispose()` is its alias. A later `update` or `digest` throws
`NEO_CRYPTO_CLOSED`. The HMAC key is a `Uint8Array` or UTF-8 string and is copied at construction;
any length, including zero, is accepted. Input is copied at `update`, so later mutation of the
caller's view does not change the result.

`hashAsync(algorithm, data[, encoding])` and `hmacAsync(algorithm, key, data[, encoding])` are the
asynchronous one-shot forms. They return a `Promise` and are genuinely nonblocking: argument
validation, budget admission, and one bounded copy of the input into a module-owned buffer run on the
isolate thread, and all hashing runs on the runtime's four-thread blocking pool through a `deno_core`
async op. The worker
builds the state, hashes the HMAC key, hashes the data, and encodes the digest; no hashing happens on
the isolate thread. They are not `Promise.resolve` wrappers around a blocking call. Input is copied
before the promise is returned, so mutating it afterwards does not change the result. The event loop
continues to run microtasks while the worker hashes.

The synchronous object accepts at most 16 MiB of input across all `update` calls; one HMAC key is at
most 1 MiB; one asynchronous call accepts at most 16 MiB of data; at most 64 asynchronous jobs may be
pending, even when they are empty; and at most 64 MiB of copied data and HMAC key bytes may be
pending. A submission over either asynchronous bound rejects with `NEO_CRYPTO_BUSY`. The per-call
byte limits are checked before the bytes are copied, and a rejected update adds no prefix. A refused
Rust allocation is `NEO_CRYPTO_LIMIT`; allocation failure inside V8 or the JavaScript heap is not
guaranteed to be recoverable.

At most 1024 incremental hash or HMAC objects may be open per runtime. Creating past that throws
`NEO_CRYPTO_TOO_MANY_OBJECTS`. There is no finalizer: an object that is created and abandoned keeps
its slot until `close()`/`dispose()`, `digest()`, or the runtime is dropped. The hard cap bounds
abandoned objects, and explicit disposal releases a slot deterministically. An asynchronous job's
job and byte reservations are released when the worker finishes, panics, or fails to start. A job
aborted before it starts on a blocking thread releases them immediately; a job that already started
cannot be stopped and stays charged until it returns. Abandoning the returned JavaScript `Promise`
does not cancel anything: the wrapper starts the join op immediately, and an unawaited promise still
lets the job finish. Dropping only the `run_file_async` entry future also stops polling but does not
drop the runtime's op futures, so it is not cancellation. Tearing down the runtime drops those
futures and unjoined resources, aborting only a job that has not started. See the cancellation
paragraph in the PBKDF2 and bcrypt section for the shared lifetime.

Input views use the same typed-array validation as the envelope: a `Proxy`, a lookalike object, or a
different typed-array kind is a `TypeError`; a detached view is `NEO_CRYPTO_DETACHED`; offset views,
`SharedArrayBuffer` views, and resizable-buffer views are accepted, with only the view's own bytes
read. There is no `Symbol.hasInstance` or `byteLength` hook. A lone surrogate in a string becomes
U+FFFD, as it does in Node's UTF-8 encoding.

**PBKDF2 and bcrypt.** `pbkdf2(algorithm, password, salt, iterations, length)` derives `length` raw
bytes synchronously and returns a `Uint8Array`. `password` and `salt` are exact `Uint8Array` bytes or
strings encoded as UTF-8. The accepted algorithms are `sha256`, `sha384`, and `sha512`, compared
case-insensitively; SHA-1 and other names throw `NEO_CRYPTO_ALGORITHM`. `iterations` is an integer
from 1 through 1,000,000, `length` is an integer from 1 through 1024 bytes, and one password or salt
is at most 1 MiB. These are neojs limits, not PBKDF2 limits. A string is measured by its UTF-8
length before it is copied. `pbkdf2Async` takes the same arguments and returns a `Promise`; the
derivation runs on the bounded blocking pool.

`bcryptHash(password[, cost])` returns a 60-character modular-crypt string in the `$2b$` format. The
password is exact bytes or a UTF-8 string. At most 72 bytes are accepted; a longer input throws
`NEO_CRYPTO_LIMIT` and is never silently truncated. Exactly 72 bytes are hashed as 72 bytes with no
added NUL. `cost` is an integer from 4 through 16, defaults to 10, and is bounded above by neojs so
one call cannot ask for an impractical work factor. The 16-byte salt comes from the operating system
CSPRNG; an entropy failure is `NEO_CRYPTO_RANDOM` and never falls back to predictable bytes.
`bcryptVerify(password, hash)` accepts `$2a$`, `$2b$`, and `$2y$` hashes. It returns `false` for a
malformed string, an unsupported version (`$2x$` included), a cost outside 4 through 16, or a
password longer than 72 bytes. It does not compute a hash for a malformed or over-cost input, so a
malicious hash cannot force an unbounded work factor. The comparison inside the bcrypt primitive is
constant-time. `bcryptHashAsync` and `bcryptVerifyAsync` are the asynchronous forms; a well-formed
verification runs on a worker, while an oversized password or malformed/over-cost hash resolves to
`false` without starting a worker because there is no expensive work to do.

PBKDF2 and bcrypt jobs share the hash executor and its bounds: the same four-thread blocking pool,
at most 64 pending jobs, and at most 64 MiB of copied input. A submission over either bound rejects
with `NEO_CRYPTO_BUSY`. Cancellation is best effort and is not triggered by abandoning the returned
`Promise`: the wrapper starts the join op immediately, so an unawaited promise still runs the job to
completion. Dropping only the `run_file_async` entry future stops polling but does not drop the
runtime's op futures or resources, so it is not cancellation. Tearing down the runtime drops the
pending join-op futures and unjoined job resources, aborting a job that has not started on a blocking
thread and releasing its reservation. A job that already started cannot be stopped. Abort is a no-op
for it, its zeroizing input copies and reservations stay charged until it returns, and its result is
discarded if the join future is gone. A worker's random-source, limit, or internal failure category
is carried through the join buffer rather than flattened into an internal error. `pbkdf2Async`,
`bcryptHashAsync`, and
`bcryptVerifyAsync` need a Tokio current-thread runtime to host deno_core's async-op driver, exactly
like `hashAsync`; any other executor rejects with `NEO_CRYPTO_NO_EXECUTOR` before copying input or
starting a worker. The synchronous forms work on every executor.

PBKDF2 is tested against RFC 7914's PBKDF2-HMAC-SHA256 vectors and independently reproduced
SHA-384/SHA-512 and NUL/Unicode fixtures. bcrypt is tested against published Apache htpasswd,
jBCrypt, and Go bcrypt vectors, including byte-for-byte hash generation with the published Go salt,
malformed and over-cost hashes, NUL/Unicode input, 72/73-byte boundaries, and a long two-byte string
ending in an unpaired surrogate checked against explicit U+FFFD bytes. Password and salt destination
buffers owned by this module are cleared with `zeroize` when the operation finishes. A string is
converted directly into that destination by V8's UTF-8 writer with U+FFFD replacement, avoiding a
module-owned intermediate Rust string. V8's internal string storage and flattening, conversion
storage inside V8 or its dependencies, the bcrypt crate's internals, PBKDF2/HMAC state, and the
JavaScript heap copies are outside this guarantee. This is best-effort clearing, not a complete
zeroization promise. Errors never include a password, salt, derived key, or bcrypt hash.

The primitives are the RustCrypto `sha2` and `hmac` crates, tested against published FIPS 180-4 and
RFC 4231 vectors, in Rust unit tests and in JavaScript integration tests for one-shot and unevenly
chunked incremental input. This is a small neojs surface, not Node's `crypto`: it does not implement
algorithms outside the three names, `copy()`, callbacks, `createCipheriv`, or WebCrypto's `subtle`.
It also does not provide Node's `pbkdf2`/`pbkdf2Sync` or a Node bcrypt package API, and its argument
names, encodings, limits, and Promise forms are neojs-specific. It does not promise complete
zeroization: key, input, and digest-state bytes may remain in Rust buffers and the JavaScript heap
until reclaimed.

**Shamir secret sharing.** `shamirSplit(secret, threshold, shareCount)` splits a `Uint8Array`
secret into `shareCount` shares over GF(256), each a frozen `{ index, value }` record whose `value`
is a `Uint8Array` of the secret's length. `threshold` and `shareCount` are integers from 2 through
255 with `shareCount >= threshold`; a secret is at most 8 MiB, and `secret.length * shareCount` is
at most 64 MiB. Field arithmetic uses reduction polynomial `0x11b` with generator `0x03` and
one-based indexes, so share values interoperate bytewise with the Neotales PLAB .NET and
rust-libraries Shamir encodings; the JavaScript record shape is not a PLAB wire format. Every
non-constant coefficient comes from the operating system CSPRNG, and an entropy failure throws
`NEO_CRYPTO_RANDOM` rather than returning predictable shares. `shamirCombine(shares[, threshold])`
interpolates at `x = 0` and returns a new `Uint8Array`; with `threshold` it requires at least that
many shares, and it always requires at least two. Shares are not authenticated: tampered shares,
shares mixed from different splits, a wrong index, or fewer than the original threshold reconstruct
a different byte string, often without an error. Compare the result with an authenticated envelope
or an out-of-band digest when a wrong reconstruction must be detected. A `SharedArrayBuffer` view is
rejected with `NEO_CRYPTO_SHAMIR_SHARED` because copying it is not one coherent snapshot.
`shamirSplitAsync` and `shamirCombineAsync` are the `Promise` forms: validation and one bounded copy
run on the isolate thread and the arithmetic runs on the bounded blocking pool, sharing the 64-job
and 64-MiB budget with `hashAsync`. Both need the same Tokio current-thread runtime as the other
asynchronous crypto calls.

**Scope of this slice.** This is a neojs-specific AES-256-GCM envelope, bounded stateful
AES-256-CBC/AES-256-GCM cipher, randomness, SHA-2, HMAC, PBKDF2, bcrypt, and GF(256) Shamir secret
sharing surface, not Node's `crypto` module and not the Web Crypto API. `crypto.subtle` (and with it
`subtle.encrypt`, `subtle.decrypt`, `subtle.digest`, and `subtle.generateKey`),
`createCipheriv`/`createDecipheriv`,
Node's full `Cipher` contract, AES modes other than CBC and GCM, other KDFs,
PHC/MCF/`crypt(3)` password-string parsing, and a PLAB .NET Crypto-compatible encrypt-then-MAC
surface are **not implemented**. The envelope key generator is
`generateEnvelopeKey`. `generateKey` is deliberately **not** exported in this slice: the name is
reserved for a future Node-compatible surface, whose `generateKey` has different semantics from the
envelope generator. Asynchronous neojs APIs are `Promise`-based, not callback-based. WebCrypto's
`subtle.generateKey` is also separate. Node-like names such as `createCipheriv`, WebCrypto's
`subtle`, and any PLAB-compatible surface are additive and separate; do not treat this surface as a
Node or PLAB compatibility layer. The slice is unreleased, so the names can still be adjusted if the
expansion needs one of them.

The envelope functions, the stateful cipher objects, the incremental hash and HMAC objects,
`pbkdf2`, `bcryptHash`, `bcryptVerify`, `shamirSplit`, and `shamirCombine` are **synchronous**.
None of them returns a `Promise` or takes a callback.
`hashAsync`, `hmacAsync`, `pbkdf2Async`, `bcryptHashAsync`, `bcryptVerifyAsync`,
`shamirSplitAsync`, and `shamirCombineAsync` are the asynchronous forms: they return a `Promise` and
do real work on the bounded blocking pool, rather than wrapping blocking work in an
already-resolved `Promise` to look nonblocking. Under
`Runtime::run_file_async`, asynchronous hashing, password hashing, and Shamir work need a Tokio
current-thread runtime to host deno_core's async-op driver; a non-Tokio executor or a multi-thread
Tokio runtime gets `NEO_CRYPTO_NO_EXECUTOR` instead. The synchronous API works on every executor.
`Runtime::run_file` drives the async-op driver, so asynchronous hashing works there. There is no
secure-context or TLS condition: the module is available in every runtime.

**UUID generation.** `Neo.getBuiltInModule("neo:uuid")` returns a frozen module object that is cached
for the realm. `neo:uuid` is the canonical and only module specifier. The named exports are `v4`
and `v7`, and each is the same function object as the matching registry property. There is no
default export on the aggregate module.

The subpath modules `neo:uuid/v4` and `neo:uuid/v7` each expose one generator as both the named
export and the default export. Their `v4`/`v7` named value and default
value are the same function objects as `uuid.v4` and `uuid.v7`, not wrappers:

```js
import { v4 as aggregateV4, v7 as aggregateV7 } from "neo:uuid";
import v4 from "neo:uuid/v4";
import { v7 } from "neo:uuid/v7";

if (v4 !== aggregateV4) throw new Error("v4 identity changed");
if (v7 !== aggregateV7) throw new Error("v7 identity changed");

console.log(v4()); // 36-character RFC 9562 version 4 text
console.log(v7()); // 36-character RFC 9562 version 7 text
```

The modules are available in every runtime; there is no secure-context or TLS gate.

Both functions take no arguments, are synchronous, and return canonical lowercase text: 32
hexadecimal digits in groups of 8, 4, 4, 4, and 12, separated by hyphens. There is no braces,
`urn:uuid:` prefix, uppercase, binary, or alternate output form. A non-zero argument count, including
a trailing `undefined` or a callback, throws `TypeError` before the host op runs. No Promise is
returned and no callback form is reserved.

`v4()` fills 128 bits from the operating system CSPRNG, then sets the four version bits to `0100` and
the two variant bits to `10`. The remaining 122 bits are random. A version 4 UUID carries no time,
node, or sequence information, so its values have no order and must not be used as a sort key.

`v7()` uses the RFC 9562 layout: 48 big-endian bits of Unix time in milliseconds, the version `0111`,
12 random bits (`rand_a`), the variant `10`, and 62 random bits (`rand_b`). The module uses the
random-payload method and keeps no per-process counter. Ordering is therefore only what the
timestamp provides:

- Values generated in different milliseconds sort by the embedded timestamp when the same system
  clock moves forward.
- Values generated within one millisecond carry random low bits and have no defined order relative
  to one another.
- A backward system clock step produces a value that sorts before earlier ones. Values from
  different processes or machines are not ordered.

This module does **not** promise monotonicity, uniqueness, or a total order. Use a database sequence,
a Lamport clock, or an explicit ordering column when the application needs one. The 74 random
payload bits make a same-millisecond collision unlikely, not impossible.

The 48-bit timestamp is a format limit. A current time before the Unix epoch, or at or beyond 2^48
milliseconds (roughly the year 10889), fails with `NEO_UUID_CLOCK` instead of wrapping, truncating, or
clamping. The clock is read before the CSPRNG, so an unrepresentable time consumes no entropy. An
operating-system CSPRNG failure is `NEO_UUID_RANDOM`; there is no `Math.random`, time-derived, or
other insecure fallback. Messages are fixed and never include entropy or a clock value. A random
failure also has a non-enumerable `cause` string built only from a numeric OS error code or one of
`getrandom`'s fixed categories (for example `getrandom: OS error 5`); the crate's display text,
entropy bytes, UUID bytes, and clock values are not exposed.

The RFC 9562 bit layout, version and variant setting, and formatting come from the MIT OR
Apache-2.0 `uuid` crate's `Builder`. Its convenience constructors `Uuid::new_v4` and `Uuid::now_v7`
are deliberately not used because they panic when `getrandom` fails; neojs calls `getrandom::fill`
itself and passes the bytes to the builder, so an entropy failure is a returned error. The Rust unit
tests reproduce the RFC 9562 Appendix A.4 version 4 example and Appendix A.7 version 7 example byte
for byte, check the version and variant fields, and cover the epoch and 48-bit overflow boundaries.
The JavaScript tests cover registry and import identity, canonical output, uniqueness, argument
strictness, and an injected entropy failure that verifies the stable code and the fixed `cause`
diagnostic.

This slice implements generation only. It does not parse or validate UUID text, extract a timestamp,
generate nil or max UUIDs, or implement versions 1, 3, 5, 6, or 8. It reads no file, environment
variable, network, or process state beyond the system clock, installs no global, and is not a
sandbox. It is separate from Node's `crypto.randomUUID()`, which has its own API and produces only
version 4 values.

**Planned, not implemented.** The `@neotales/chars` reference also defines the `...At` string-index
helpers, the `...Unsafe` variants, `equalFold`, `simpleFold`, and character-code constants. The
QuickJS `neo:chars` module uses the same broad shape without the `Unsafe` variants. None of those
are implemented here, and other built-in module names are not specified; `Neo.getBuiltInModule`
returns `undefined` for them rather than promising a future module, and an import of one is an
error. Import maps, module type attributes, and a default export for the `chars` built-in are also
not implemented. The `console` built-in does have a default export; see above.

**Embed the runtime.** `neojs_core` exposes the same loader and console to Rust callers.

```rust
use neojs_core::Runtime;

fn main() -> Result<(), neojs_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("hello.js")?;
    Ok(())
}
```

Use `Runtime::run_file_async` inside an existing Tokio runtime, or on any other executor. A runtime
owns a Tokio driver with one worker thread for V8's delayed foreground tasks and the bounded
`crypto` blocking pool, plus a current-thread runtime that drives `run_file` and deno_core's
async-op driver, so the async form neither needs nor borrows its caller's runtime. It does not
offload work: entry and imported files
are read and type-stripped synchronously on the thread that polls the future, and module compilation
and evaluation run there too. `crypto`'s `hashAsync`, `hmacAsync`, `pbkdf2Async`,
`bcryptHashAsync`, and `bcryptVerifyAsync` additionally need a Tokio current-thread runtime to host
the async-op driver; they work under `run_file`, and under `run_file_async` only when the caller's
executor is a Tokio current-thread runtime. Otherwise they throw `NEO_CRYPTO_NO_EXECUTOR`; the
synchronous hash, HMAC, PBKDF2, and bcrypt API still works.
`Runtime::run_file` returns `Error::BlockingInAsyncContext` inside a
Tokio runtime instead of blocking, unless the runtime is already consumed. The module can write to
stdout and stderr through `console`; `run_file` itself prints nothing. The async form does work only
when polled. Dropping it before the first poll leaves the runtime unused, and so does a preflight
failure such as an unsupported format or unreadable entry. Once loading starts, cancelling it leaves
the runtime consumed, and the module may already have run and printed. Create the `Runtime` on the
thread that will use it; it is not `Send`.

## Design

The [ECMAScript npm modules proposal](docs/esm-npm-proposal.md) is a design proposal for review,
not implemented behavior.

The [`neojs-dbapi` crate](neojs-dbapi/README.md) is the SQLite-only database foundation; other
database providers and a JavaScript/Node adapter are not yet wired in. See the
[database API notes](docs/db-api.md) for its scope and design decisions.

The [`neojs-manifest` crate](docs/neo-jsonc.md) is the bounded foundation for `neo.jsonc`: strict
JSONC parsing, schema validation, and canonical import-specifier normalization for `gh:`,
`github:`, `npm:`, `jsr:`, and `node:`. It does not install an import map, expand workspaces, or
apply `include`/`exclude`. A normalized `gh:`/`github:` coordinate is the raw HTTPS URL the V8
runtime loads when remote loading is enabled; the runtime also loads `jsr:` imports only then. The
manifest crate does not change either decision, and `npm:` and `node:` remain rejected at runtime.

### QuickJS runtime: neojs-lite-core

`neojs-lite-core` is the primary, embeddable QuickJS runtime, with `neojs-lite` as a secondary
command-line wrapper. They are separate workspace members and are not part of the default build:
`cargo build` neither compiles QuickJS nor puts `rquickjs` in the V8 dependency graph.

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

Build the embeddable crate or the command explicitly:

```sh
cargo build -p neojs-lite-core
cargo run -p neojs-lite-core --example embed -- examples/hello.js
cargo build -p neojs-lite
```

The QuickJS runtime currently runs `.js`, `.mjs`, and extensionless ECMAScript modules, resolves
relative imports and `file:` URLs, and awaits top-level `await` that settles from QuickJS's job
queue. It then drains queued jobs, bounded at 100,000, so trailing `.then` callbacks and unawaited
dynamic imports still run; a nonconverging job chain or a throwing queued job fails closed, and a
promise still rejected without a handler when the queue empties is an error; rejection diagnostics use
primitive values and an Error's own data without invoking getters, proxies, `toString`, or `String`.
It does **not** implement TypeScript, host asynchrony or timers, most `neo:` built-in modules,
permissions, or a sandbox, and it does not claim parity with the V8 runtime. It installs a minimal
`Neo` global whose engine is `"neo-lite"`; with the default `neo-chars` feature its built-in module
list is the canonical `["neo:chars"]`. `Neo.getBuiltInModule("neo:chars")` returns the frozen
registry object whose named functions are the same function objects the `neo:chars` namespace
exports; the namespace itself is a distinct object. With `default-features = false` the list is empty.
A bare name, another scheme, or any other specifier returns `undefined`, and a non-string name throws
`TypeError`. The `chars` implementation and canonical wrapper come from the engine-neutral
`neojs-builtins` crate, which both runtimes use; it has no V8 or QuickJS dependency. Both runtimes
share `neojs-module-policy` for executable-extension decisions and rejection reasons, so `.cjs`,
JSX/TSX, declaration files, JSON, WebAssembly, and native addons fail with the same explanation in
either.

### Binary size tracking

`eng/size/` measures the release-profile `neojs` and `neojs-lite` executables and
appends each result to an append-only JSONL ledger. A record carries the exact
bytes and SHA-256, the source state, artifact, target, toolchain, requested
features, Cargo and lockfile inputs, and explicitly labels effective features,
linker and V8-archive provenance as captured or unknown. Strict comparison
series and a labelled trend report keep a toolchain or config change from being
read as a source-code delta; a comparison with any uncaptured control is marked
controls-incomplete rather than strict. The directory is self-contained: see
[`eng/size/README.md`](eng/size/README.md) for the methodology and
`eng/size/check.sh` for the deterministic checks.

## Limits

Implemented: ECMAScript modules, erasable TypeScript in `.ts` and `.mts` files, top-level await,
local module loading, opt-in HTTPS module loading with explicit limits, opt-in `gh:`/`github:`
specifier loading, opt-in `jsr:` resolution
through the public JSR registry with manifest length and SHA-256 verification, imports of the
`age`, `assert`, `chars`, `console`, `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`,
`strings`, `test`, and `uuid` built-ins by their canonical `neo:` specifiers, console output,
`neojs test` over
multiple files and globs, bounded in-memory byte streams, live mutable and read-only array and
typed-array views, mutable and read-only Unicode code point buffers, UTC five-field cron parsing and
in-process scheduling, writes to the process's stdout and stderr, lexical POSIX and Windows path
operations, SHA-2 hashing and HMAC (synchronous incremental and Promise-based one-shot), synchronous
SSH command
execution with mandatory host-key verification and password, in-memory-key, or explicit Unix-agent
authentication, the built-in module registry and its `Neo.engine` and `Neo.getBuiltInModulesList`
introspection, and the embedding API above. The QuickJS runtime installs only the `Neo` introspection
surface, not the built-in modules: `Neo.engine` is `"neo-lite"`, its built-in module list is empty,
`Neo.getBuiltInModule` returns `undefined` for every string name, and a non-string name throws
`TypeError`. Features outside this list are not implemented.

- TypeScript has no type checking and no transpilation. Only syntax that can be erased is accepted:
  `enum`, `namespace`, parameter properties, `import =`, `export =`, decorators, and angle-bracket
  type assertions are rejected. `.tsx`, `.jsx`, `.cts`, `.d.ts`, `.d.mts`, and `.d.cts` are
  rejected by extension. CommonJS is rejected by the `.cjs` and `.cts` extensions; a `.js` file
  that uses `require`, `module`, or `exports` fails as JavaScript. Transpile anything else to an
  ECMAScript module first.
- `node:`, `npm:`, `http:`, `data:`, and `blob:` imports are rejected. `gh:` and `github:`
  imports are rejected unless the runtime is created with remote loading enabled (`neojs
  --allow-remote`, or `NEOJS_ALLOW_REMOTE=1`); when enabled, a validated specifier is rewritten to
  one credential-free `https://raw.githubusercontent.com/owner/repo/ref/path` URL and loaded as
  that HTTPS module. The legacy `gh:owner/repo/path/@ref` spelling is accepted. The reference is
  one segment, so a branch name containing `/` cannot be written; use a tag or a full commit SHA.
  GitHub is not an allowlist and not a sandbox. `jsr:` imports are
  rejected unless the runtime is created with remote loading enabled (`neojs --allow-remote`, or
  `NEOJS_ALLOW_REMOTE=1`); when enabled they resolve through the public JSR registry, and each file
  is checked against the version manifest's length and SHA-256 before it runs. The checksum proves
  consistency with the retrieved metadata, not publisher identity; the cache is mutable local state,
  so an attacker who rewrites both the cached manifest and the cached body can keep them consistent.
  A direct `https://jsr.io/...` import from a local or ordinary remote module is an ordinary HTTPS
  module, not a `jsr:` import, and does not use registry metadata or checksums; an explicit jsr.io
  URL imported inside a JSR graph is verified against the manifest. One exception: if the same
  canonical URL is already pending from a `jsr:` resolution (including a bare `import.meta.resolve`
  or a failed verified load), a later direct load of that URL takes the verified path because the
  original specifier is not recoverable at load time. That is conservative verification, not a
  bypass. JSR support covers
  exported entrypoints and the relative modules they import; `npm:`, `node:`, dist-tags,
  private registries, authentication, import maps and `neo.jsonc`, lockfiles, offline policy, and
  publisher signatures are not implemented. A JSR package that imports one of those fails at
  resolution. The first metadata request for a package or version is synchronous and blocks the
  isolate thread. `https:` imports are
  rejected unless the runtime is created with remote loading enabled (`neojs --allow-remote`, or
  `NEOJS_ALLOW_REMOTE=1`), and `http:` is always rejected.
- With the off-by-default `node-compat` feature, a runtime created with
  `Runtime::with_node_compat` accepts only `node:child_process`, and only its synchronous
  `spawnSync` and `execFileSync` with `encoding: "utf8"`; other `node:` modules and every other
  form are rejected with a reason. Node modules are not added to the `Neo` registry, no bare
  `child_process` alias exists, and `node:test` is never an alias for the `test` built-in.
  Serialized bridge requests are capped before parsing, guest serialization uses
  install-time intrinsics and null-prototype records, and a timeout or inherited pipe is followed
  by a bounded drain and process-group kill; an inherited pipe is an explicit incomplete error,
  not a success. See [`docs/node-compat.md`](docs/node-compat.md). The built-in modules are
  `age`, `assert`, `chars`, `console`, `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`,
  `strings`, `test`, and `uuid`,
  reached through `Neo.getBuiltInModule("neo:age")`,
  `Neo.getBuiltInModule("neo:assert")`, `Neo.getBuiltInModule("neo:chars")`,
  `Neo.getBuiltInModule("neo:console")`, `Neo.getBuiltInModule("neo:cron")`,
  `Neo.getBuiltInModule("neo:crypto")`, `Neo.getBuiltInModule("neo:os")`,
  `Neo.getBuiltInModule("neo:path")`, `Neo.getBuiltInModule("neo:slices")`,
  `Neo.getBuiltInModule("neo:ssh")`, `Neo.getBuiltInModule("neo:stdio")`,
  `Neo.getBuiltInModule("neo:strings")`, `Neo.getBuiltInModule("neo:test")`,
  `Neo.getBuiltInModule("neo:uuid")`,
  `Neo.getBuiltInModule("neo:uuid/v4")`, and `Neo.getBuiltInModule("neo:uuid/v7")`, or
  through the import specifiers `neo:age`, `neo:assert`, `neo:chars`, `neo:console`,
  `neo:cron`, `neo:crypto`, `neo:os`, `neo:path`, `neo:slices`, `neo:ssh`, `neo:stdio`,
  `neo:strings`, `neo:test`, `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7`. Other `neo:` modules
  are rejected by name. Bare module specifiers, including a bare built-in name such as `age`, are
  rejected; the message names the canonical `neo:` form. Import maps are not supported.
- Module type attributes are rejected. `with { type: "json" }`, `with { type: "text" }`, `with {
  type: "bytes" }`, and custom types fail with a reason on static and dynamic imports, including for
  built-in modules. Only untyped ECMAScript modules are loaded. Attributes that do not name a type
  are not interpreted by neojs.
- JSON and WebAssembly module files are rejected, both as the entry file and as imports. V8's
  `WebAssembly` global is present but untested.
- There is no permission system. The loader reads any file the process can read. neojs is not a
  sandbox. `chars`, `console`, `os`, `slices`, and `strings` are plain ECMAScript and add no
  capability beyond the globals they expose; `os` holds two build-target constants and reads no
  host state, the `console` module writes to the process streams already used by the global
  console, and `slices` aliases Arrays and typed arrays the caller already has; it creates no
  buffer, opens no file, and is not a snapshot or an isolation boundary. `assert` calls
  one host op that observes a genuine native Promise's settlement by
  attaching settlement handlers without reading a user `then` or consulting `Symbol.species`; it
  adds no other capability. `cron` calls six host ops: it parses and validates an expression, stores
  one schedule per job in the resource table, reads the host clock, computes the following
  occurrence, sleeps on the runtime's Tokio timer, and cancels a job. It reads no file, environment,
  network, or process state and adds no capability beyond keeping the event loop alive until the
  job is disposed. The scheduler behind it is a UTC five-field implementation; it has no local-time,
  DST, time-zone, backoff, or fake-timer surface, and `test-support` clock hooks are absent from a
  normal build. `test` is implemented in the realm and registers no private host op; it
  uses the runtime's existing print, timer, and event-loop primitives and adds no filesystem,
  environment, network, or process authority. It runs test callbacks in this realm, so it is not
  isolation or a sandbox.
  `age` calls four host ops for X25519 identity generation and binary age v1 encryption/decryption.
  `crypto` calls thirty-two host ops: four for the operating system CSPRNG and AES-256-GCM, eight
  for bounded stateful AES-256-CBC/AES-256-GCM ciphers, seven for SHA-2 hashing and HMAC, seven for
  PBKDF2 and bcrypt, and six for GF(256) Shamir secret sharing, including their bounded
  asynchronous paths. `uuid` calls two host ops for RFC 9562 version 4 and version 7 generation;
  both draw from the operating system CSPRNG, and a failed draw returns an error rather than falling
  back. `ssh` calls three
  host ops: it performs DNS and outbound TCP, completes the SSH handshake, verifies an explicit
  Ed25519 host-key pin or explicit `known_hosts` text, authenticates with exactly one of a password,
  an in-memory private key, or an explicit Unix `agent: true`, and runs one command at a time. It has
  no unknown-host mode and reads no identity file or default known-hosts file. Password and
  private-key modes read no environment; only the explicit agent opt-in reads `SSH_AUTH_SOCK`, and
  only after the host key is trusted. Agent authentication tries at most 16 identities, is Unix-only
  (Windows pipes are rejected), and never forwards an agent. The agent timeout is not a hard wall
  clock: `UnixStream::connect` may block before any timeout is installed, and the backend's
  30-second read/write timeout is inactivity-based, so a trickling peer can prolong enumeration or a
  signing request. It is separate from `connectTimeoutMs`, whose deadline is checked before and after
  the SSH phases. The module adds
  no filesystem, process, stdin, or permission capability. Its two blocking calls carry documented
  deadlines; there is no asynchronous form. `stdio` calls four
  host ops: it copies or UTF-8 encodes bounded byte data, copies a byte range out of a private
  snapshot, and writes to the process's own stdout and stderr descriptors, which the console already
  uses. It adds no filesystem, process, environment, network, or stdin access. `path` calls two host
  ops: one reports the build target's path dialect, and one returns the process working directory as
  a UTF-8 string or `null`. The cwd op is used only when `resolve`, `relative`, or Windows
  `toNamespacedPath` lacks a complete explicit base. `path` reads no file, environment variable,
  per-drive directory, network, or process table, and it is not a containment check. None of these
  modules manages keys or provides a permission or isolation boundary.
- There is no REPL, stdin input, worker, timer, `fetch`, `URL`, or `TextEncoder`. V8 supplies the
  ECMAScript built-ins.
- `Deno.core` internals are reachable from JavaScript. They are not a supported API.
- A `Runtime` runs one entry module. Once loading starts, another call returns
  `Error::RuntimeConsumed`, including after a load or evaluation failure. Create a new `Runtime` for
  each entry module.
- There is no timeout or interrupt. A JavaScript loop that never exits hangs the process.
- neojs defines five console methods. `log`, `info`, and `debug` write to stdout; `warn` and `error`
  write to stderr. Objects print as JSON, typed arrays print their first ten elements, and format
  specifiers such as `%s` are not interpreted. The global console object also carries other methods
  from `deno_core`; their behaviour is not part of neojs's documented surface.
- Exit codes: `0` the module finished, or every selected test file ran with no failures; `1` the
  module failed to compile, resolve imports, link, or run, or a test failed, was pending, left an
  unhandled rejection or unsettled event-loop work, or registered no tests; `2` neojs could not
  start (bad arguments, unreadable or unsupported entry file, or a test file/glob that could not be
  expanded).

Built on `deno_core` 0.413.0, with type stripping by `deno_ast` 0.53.3, test-file expansion by
`glob` 0.3.4, AES-256-GCM by
`aes-gcm` 0.10.3, PBKDF2 by `pbkdf2` 0.13.0, bcrypt by `bcrypt` 0.19.3, UUID layout and formatting by
`uuid` 1.26.1, binary age v1 by
`neotales-age` 0.0.0-alpha.1, and SSH-2 by `neotales-ssh` 0.0.0-alpha.1. Verified on Linux x86-64.
Other platforms are unverified.

## Licence

[MIT](LICENSE.md)

`neojs-core/src/crypto.rs` adapts the envelope and core encryption logic, and several test cases,
from the MIT-licensed `neo:crypto` module in the NeoLiteJS project, Copyright (c) 2026 NeoTales
contributors. `neojs-core/src/age.rs` adapts the JavaScript-facing surface, limits, and test cases
from the MIT-licensed `neo:age` module in the same project. The `assert` module adapts comparison,
cycle tracking, value formatting, and assertion-error structure from the MIT-licensed `neo:assert`
module in the same project; the adaptation renames the equality surface, adds identity and Promise
forms, and changes Map-entry rollback and callback handling. The `test` module adapts the
registration queue, failure bookkeeping, report shape, and prototype-independent append helpers from
the MIT-licensed `neo:test` module in the same project; the adaptation adds async callbacks,
per-test timeouts, pending tests, unhandled-rejection capture, a bounded event-loop settle check,
and the host report. The `stdio`
module's API shape and error
conventions are informed by the MIT-licensed `neo:io` and `neo:process` modules in the same project;
its implementation and tests are original, so no additional notice is required for it. The `path`
module adapts lexical algorithms from the MIT-licensed `neo:path` module in NeoLiteJS and from
`@neotales/path`; `neojs-core/src/builtins/neo.js` removes the glob and file URL helpers, adds the
two-namespace host-selection shape, validation, the NUL rule, and the cwd policy. The `slices`
module's API shape is informed by the MIT-licensed `@neotales/slices` package; its implementation
and tests are original and do not port that package's code, so no additional notice is required for
it. The exact upstream
revision, adapted-file list, and MIT notices are in `neojs-core/THIRD_PARTY_NOTICES.md`. The `os`
built-in is original; its two constant names are informed by the MIT-licensed NeoLiteJS `neo:os`
module, but no code is copied and no additional notice is required. The `age` format and X25519
handling come from the MIT-licensed `neotales-age` crate, Copyright (c) 2026 Neotales and x0craft. The `ssh`
built-in uses the MIT-licensed `neotales-ssh` crate, Copyright (c) 2026 Neotales and x0craft, for the
SSH-2 protocol; `neojs-core/src/ssh.rs` adds only the JavaScript-facing slice and policy described in
that crate's notice. The `crypto` hash and HMAC primitives use the RustCrypto `sha2` 0.11.1 and
`hmac` 0.13.0 crates, dual-licensed MIT OR Apache-2.0. PBKDF2 uses the RustCrypto `pbkdf2` 0.13.0
crate and bcrypt uses the MIT-licensed `bcrypt` 0.19.3 crate (with the RustCrypto `blowfish` 0.10.0
crate), all already linked transitively or added at reviewed versions; `neojs-core/src/crypto_hash.rs`
and its `password` child module add the JavaScript-facing lifecycle, encodings, bounds, work-factor
policy, and asynchronous executor, and do not modify the crates. The `uuid` built-in uses the
`uuid` 1.26.1 crate, dual-licensed MIT OR Apache-2.0, for RFC 9562 bit layout, version and variant
setting, and canonical formatting; `neojs-core/src/uuid.rs` adds the fallible operating-system
CSPRNG draw, the 48-bit clock policy, the no-monotonicity contract, the host ops, and the JavaScript
error mapping, and does not modify the crate. The full upstream MIT copyright and
permission notices, and the RustCrypto and `uuid` licence references, are reproduced in
`neojs-core/THIRD_PARTY_NOTICES.md`, which ships with the `neojs-core` package. The project licence remains the MIT licence in `LICENSE.md`,
Copyright (c) 2026 NeoTales and x0craft; the crate directories carry identical copies so each package
ships it. No PLAB code is included.
