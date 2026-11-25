---
title: Quickstart
description: Run your first TypeScript script, execute tests with vael:test, emit .d.ts declarations, and configure vael.jsonc.
---

VaelJS executes ECMAScript modules (`.js`, `.mjs`) and erasable TypeScript (`.ts`, `.mts`) directly with no build or bundling step.

## 1. Run Your First TypeScript Script

Create `hello.ts` using standard erasable TypeScript syntax and canonical `vael:*` imports:

```ts
import { platform, arch } from "vael:os";
import { v7 } from "vael:uuid";

interface Greeting {
  target: string;
  id: string;
}

const msg: Greeting = {
  target: `${platform}/${arch}`,
  id: v7(),
};

console.log(`Hello from ${Vael.engine} on ${msg.target} (${msg.id})`);
```

Run it on either runtime:

```sh
vjs hello.ts
vjs-lite hello.ts
```

:::note[Erasable TypeScript Only]
VaelJS strips type annotations, `interface`, `type`, generics, `as`, `satisfies`, and `import type` without type-checking. Non-erasable transforms (`enum`, `namespace`, parameter properties, decorators) and `.tsx`, `.jsx`, `.cts`, and `.d.ts` files are rejected.
:::

## 2. Run Tests with `vjs test`

Create `math.test.ts` using `vael:test` and `vael:assert`:

```ts
import assert from "vael:assert";
import test from "vael:test";

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

test("clamp bounds values cleanly", () => {
  assert.strictEquals(clamp(15, 0, 10), 10);
  assert.strictEquals(clamp(-3, 0, 10), 0);
  assert.equals({ ok: true }, { ok: true });
});
```

Run one or more files or case-sensitive globs:

```sh
vjs test math.test.ts
vjs-lite test "*.test.ts"
```

Each selected file executes in a **fresh isolated runtime**, so module globals and test queues cannot leak across files. There is no default test discovery—you must pass at least one path or glob, and a file that registers zero tests fails with exit code `1`.

## 3. Generate TypeScript Declarations (`vjs types`)

Emit `.d.ts` files matching your runtime profile for editor autocomplete and `tsc --noEmit` checks:

```sh
# Write the full directory layout for vjs (V8)
vjs types --out-dir ./types

# Or emit declarations for vjs-lite (QuickJS)
vjs-lite types --out-dir ./types

# Or stream a single-file bundle to stdout
vjs types > vjs.d.ts
```

Point a minimal `tsconfig.json` at the generated declarations:

```json
{
  "compilerOptions": {
    "target": "ES2024",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "noEmit": true,
    "strict": true,
    "erasableSyntaxOnly": true,
    "types": ["./types/index.d.ts"]
  }
}
```

## 4. Add a `vael.jsonc` Manifest

Create `vael.jsonc` in your project root to map clean specifiers or define workspace packages:

```jsonc
{
  "name": "@example/app",
  "version": "0.1.0",
  "imports": {
    "@app/config": "./src/config.ts"
  }
}
```

Both `vjs` and `vjs-lite` automatically discover `vael.jsonc` upward from the entry module's directory. See [vael.jsonc & Workspaces](/cli/vael-jsonc/) for the full schema and rules.

## Next Steps

- Compare engines in [vjs vs vjs-lite](/getting-started/vjs-vs-vjs-lite/).
- Explore all CLI options in [Commands & Flags](/cli/commands/).
- Build a service in [Go-Style WebApps](/guides/webapps/) or automate tasks in [Automation Recipes](/guides/automation/).
