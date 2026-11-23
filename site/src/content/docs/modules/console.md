---
title: console
description: The realm's installed console object, its five NeoJS methods, and the mutation and identity rules.
---

`console` is available in every V8 runtime and is not available in NeoJS Lite. Import it as
`neo:console`; the registry form is `Neo.getBuiltInModule("console")`.

The default export is the console object the runtime installed. The named exports are the five
NeoJS-defined method values, read when the console module is first evaluated.

## Exports

| Export | Writes to |
| --- | --- |
| `log`, `info`, `debug` | stdout |
| `warn`, `error` | stderr |

```js
import console, { warn } from "neo:console";

console.log("stdout");
warn("stderr");
```

The runtime installs `log`, `debug`, `info`, `warn`, and `error` on the global console before any
module runs. In a realm where nothing replaced them, the named exports are those installed
functions.

## Identity and mutation

- The registry object and the default export's value are the console object the runtime installed,
  not a frozen copy. Replacing a method through the default export, the registry, or
  `globalThis.console` while they still name that object changes all three.
- Replacing the whole `globalThis.console` property does not change the registry object or the
  default export's value. The ESM module reads the registry, not `globalThis.console`.
- The named exports are captured when the console module is evaluated. A method replaced before that
  first evaluation is what the named export captures; replacing it later does not change the named
  binding. Read the property from the default export or the registry to follow later replacements.
- The ES module namespace (`import * as ns`) is not the console object. Its `default` property is the
  registry object, and its named properties are the captured function objects. Assigning to a
  namespace property throws `TypeError`.
- Adding a property to a shared function object is visible through every reference to that function.

The console object also carries whatever other properties the engine's global console has. Those
are not part of NeoJS's documented surface.

## Output shape

NeoJS defines five console methods. Objects print as JSON, typed arrays print their first ten
elements, and format specifiers such as `%s` are not interpreted.

This module is not a permission, sandbox, or isolation boundary. It writes to the same process
streams as the global console and adds no new capability.

## Related

- [`stdio`](/modules/stdio/) for bounded in-memory byte streams and direct stdout/stderr writers.
