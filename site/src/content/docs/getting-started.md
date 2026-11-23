---
title: Getting started
description: Build the NeoJS command, run a module, run TypeScript, and add your first test.
sidebar:
  order: 1
---

This page builds the V8 command from a checkout. The workspace declares Rust 1.100 as its minimum,
and the repository verifies Linux x86-64. Other targets are unverified.

## Build the command

From the repository root:

```sh
cargo install --path neojs
```

That installs the `neojs` binary. To work without installing it:

```sh
cargo run -p neojs -- hello.js
```

The QuickJS runtime is a separate, opt-in build. It is not part of `cargo build`'s default members:

```sh
cargo build -p neojs-lite
```

## Run a module

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

`neojs` reads the file as an ECMAScript module, evaluates it, and waits for top-level promises.

## Import another file

Relative paths, absolute paths, and `file:` URLs resolve from the module that imports them. Static
imports and dynamic `import()` both work.

`greet.js`:

```js
export function greet(name) {
    return `hello, ${name}`;
}
```

`main.js`:

```js
import { greet } from "./greet.js";

console.log(greet("world"));
```

```sh
neojs main.js
```

```text
hello, world
```

## Run TypeScript

`.ts` and `.mts` files run by erasing type syntax. There is **no type checking** and no
transpilation. A type error is not reported, and a program can run with values that do not match
their annotation.

`options.ts`:

```ts
export interface Options {
    name: string;
}
```

`main.ts`:

```ts
import type { Options } from "./options.ts";

const options: Options = { name: "world" };
console.log(`hello, ${options.name}` satisfies string);
```

```sh
neojs main.ts
```

```text
hello, world
```

Erasable syntax includes annotations, interfaces, type aliases, generics, `as`, `satisfies`,
non-null assertions, optional parameters, `declare` declarations, and type-only imports. Constructs
that need a transform are rejected: `enum`, `namespace`, parameter properties, `import value =
require(...)`, `export =`, angle-bracket assertions (`<T>value`), and decorators. `.tsx`, `.jsx`,
`.cts`, `.d.ts`, `.d.mts`, and `.d.cts` are rejected by extension.

## Run tests

`tests/math.test.js`:

```js
import test from "test";
import assert from "assert";

test("adds", () => {
    assert.equals(1 + 1, 2);
});

test("adds asynchronously", async () => {
    await Promise.resolve();
    assert.equals(2 + 2, 4);
});
```

```sh
neojs test tests/math.test.js
```

That command selects the file shown above and runs both tests.

Each selected file runs in a fresh runtime. A file that registers no tests fails. See
[the test module](/modules/test/) for timeouts, pending tests, unhandled rejections, and the exact
report shape.

## Add a built-in module

Built-ins are imported by a bare name or the same name under the `neo:` scheme. The two specifiers
reach the same module instance:

```js
import { v4 } from "uuid";
import { join } from "neo:path";

console.log(v4());
console.log(join("srv", "app", "data"));
```

The registry is also available as `Neo.getBuiltInModule("uuid")`. See
[built-in modules](/modules/) for the complete list and the identity rules.

## Try NeoJS Lite

The QuickJS runtime runs local `.js` and `.mjs` files and resolves relative imports. It has no
TypeScript support, no built-in `neo:` modules, no timers or host asynchrony, and no permission
system.

```sh
cargo run -p neojs-lite -- examples/hello.js
```

Embedding it from Rust:

```rust
use neojs_lite_core::Runtime;

fn main() -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new()?;
    runtime.run_file("main.js")
}
```

See [NeoJS Lite](/lite/) and [V8 and Lite compared](/v8-vs-lite/).

## Next

- [Command line](/cli/) — every flag, the environment switch, and the exit codes.
- [Built-in modules](/modules/) — the API reference.
- [Remote loading and security](/remote-security/) — what `--allow-remote` does and does not add.
- [Limits and unsupported features](/limits/) — the deliberate gaps.
