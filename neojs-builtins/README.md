# neojs-builtins

Engine-independent Neo built-in module implementations shared by the V8
runtime (`neojs-core`) and the QuickJS runtime (`neojs-lite-core`).

The crate contains ECMAScript source and metadata only. It has no `deno_core`
dependency, no `rquickjs` dependency, and no dependencies at all, so it can sit
in both dependency graphs without moving an engine across the separation
boundary. It performs no I/O.

```sh
cargo test -p neojs-builtins
cargo test -p neojs-builtins --no-default-features
cargo tree -p neojs-builtins -e normal
```

## What is shared

Each module contributes:

* an **implementation** expression that evaluates to the frozen public module
  object and captures the intrinsics it uses;
* a **canonical module wrapper** that re-exports that object from
  `Neo.getBuiltInModule("<canonical specifier>")`;
* a `BuiltinModule` descriptor with the canonical specifier, the exact export
  names, and both sources.

Each engine keeps its own realm shim and loader/registry adapter. The engine
decides where to splice the implementation and how to accept a specifier; the
implementation and wrapper text are the same in both.

## Module selection

Every shared module has its own Cargo feature. `chars` is the first.

* `neojs-core` has a positive `neo-chars` feature that enables the optional
  `neojs-builtins` dependency and its `chars` feature. A default V8 build
  enables `neo-chars`, so the existing surface is unchanged. `neo-slices`
  implies `neo-chars`, because `CharSlice` captures `chars.toUpper` and
  `chars.toLower`; disabling `neo-chars` also disables `slices` and removes both
  source files, registry entries, and loader entries. The `slices`
  implementation is a separate file included only under `neo-slices`, so a
  disabled build does not embed its bytes. A `neo-chars`-only build keeps
  `chars` and omits `slices`.
* `neojs-lite-core` has the same positive `neo-chars` feature, enabled by its
  default feature set. `default-features = false` removes the dependency, the
  implementation source, the registry object, the loader entry, and the
  introspection list entry. `neojs-lite` selects `neo-chars` explicitly with
  `default-features = false`, so the command bundle does not drift with library
  defaults.
* `neojs-builtins` itself has no default module features. A build without
  `chars` has no descriptor, no implementation bytes, and no wrapper.

Cargo features are additive and unify across the dependency graph. A build that
another dependent enables a module for contains that module even if one consumer
requested it off. Module selection is compile-time surface selection, not a
sandbox.

## `neo:chars`

The canonical specifier and registry key is `neo:chars`.

| Export | Contract |
| --- | --- |
| `isChar` | True for a Unicode scalar value (integer `0..=0x10FFFF`, excluding surrogates). |
| `isAscii` | True for a scalar value `0..=0x7F`. |
| `isLatin1` | True for a scalar value `0..=0xFF`. |
| `isControl` | Unicode category `Cc`. |
| `isDigit` | Unicode category `Nd`; other numerals are false. |
| `isLetter` | Unicode category `L`. |
| `isLetterOrDigit` | Category `L` or `Nd`. |
| `isLower` | Unicode category `Ll`. |
| `isUpper` | Unicode category `Lu`. |
| `isPunc` | Unicode category `P`. |
| `isSpace` | ASCII `\t\n\v\f\r`, space, U+0085, categories `Zs`, `Zl`, and `Zp`. U+FEFF and U+200B are false. |
| `isSymbol` | Unicode category `S`, including emoji. |
| `toLower` | One-code-point lowercase mapping; an expanding mapping returns the input. |
| `toUpper` | One-code-point uppercase mapping; an expanding mapping returns the input. |

All predicates are total: a non-number, a non-integer, a surrogate, an
out-of-range value, or `NaN` returns `false`. `toLower` and `toUpper` throw
`TypeError` for a non-number and `RangeError` for a non-scalar number.

The module calls no host op, reads no environment or file, and adds no
capability. It is not a sandbox.

## Precise limits

* **Unicode version follows the engine.** Classification uses ECMAScript
  Unicode property escapes (`\p{...}` with the `u` flag) and full case mapping
  through `String.prototype.toLowerCase`/`toUpperCase`. Both engines implement
  these, but their Unicode tables can be from different releases. A code point
  assigned in a newer release can classify differently between V8 and QuickJS.
  The shared contract tests use long-assigned code points and do not claim a
  fixed Unicode version. `isSpace` is intentionally Go's `unicode.IsSpace`
  contract, not JavaScript `\s`: U+FEFF is not whitespace.
* **Captured intrinsics, not isolation.** The implementation captures the
  intrinsics it uses when the realm shim is evaluated. Replacing them afterwards
  does not change the module. Code that replaced them before the shim ran is not
  defended against.
* **One module instance per realm.** The wrapper reads the public registry
  object and re-exports its function properties. Each named export is the same
  function object as the matching registry property, while the ES module
  namespace is a distinct object. The namespace is immutable by the language and
  the registry object is frozen. This is not a cross-realm or cross-thread
  mechanism.
* **Source, not a stable ABI.** The public Rust surface is the descriptor and
  source constants. JavaScript behavior is the contract; the exact text is not.

## Cross-engine tests

`tests/fixtures/chars_contract.js` is run by both `neojs-core` and
`neojs-lite-core`. Each test replaces two tokens with its adapter's specifier and
registry key. The fixture covers Unicode boundaries, scalar and category edge
cases, expanding case mappings, error classes, captured intrinsics, and module
import identity.

## Artifact checks

The engine test suites include artifact scans. Each scan reads its own test
executable and looks for distinctive byte sequences from the actual JavaScript
implementation, not markers added for testing. V8 checks both `chars` and
`slices`; QuickJS checks `chars`. A disabled build must not contain those
sequences, and an enabled build must. A hex-encoded control string from the
always-present shim proves the scan is reading real source bytes rather than
passing vacuously. The sequences are decoded from hex at runtime, so the test
source cannot itself supply the searched bytes.

Because the checks use implementation code, reintroducing the original inline
`slices` implementation into the monolith would fail a disabled scan even if no
marker or sentinel were reintroduced. The V8 `slices` implementation is a
separate file, `neojs-core/src/builtins/slices.impl.js`, included only under
`neo-slices`. The previous monolithic shim embedded it unconditionally and
removed it from the evaluated text at runtime; that is no longer how a disabled
build is produced.

## Integration state

* **Canonical registry contract.** The canonical registry contract makes
  `Neo.getBuiltInModule("neo:chars")` the only lookup and
  `Neo.getBuiltInModulesList()` canonical-only, and wrappers use canonical
  keys. Both engines implement it. This crate's `SPECIFIER`, `MODULE`, and
  canonical wrapper all use `neo:chars`; `neojs-core/src/builtins.rs` splices the
  implementation into the canonical realm shim, and
  `neojs-core/src/builtins/chars.esm.js` reads the canonical key. The V8-shared
  contract test uses the canonical specifier as both tokens.
* **Per-module strings branch (in flight, separate).** `strings` is being
  extracted independently behind an engine-local feature. This crate
  deliberately shares **`chars`**, not `strings`, so the two changes do not
  duplicate an implementation or touch the same source file. `chars` remains a
  separate module; `slices` captures `chars.toUpper`/`chars.toLower`, and the V8
  shim still defines `chars` before `slices`, so that dependency is unchanged.
* **No V8 registry rewrite in this change.** Extracting `chars` did not change
  the registry shape for the other V8 built-ins. Those remain canonical `neo:`
  modules owned by the V8 runtime; the shared crate contributes only the
  `chars` descriptor, implementation, and wrapper.
