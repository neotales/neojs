# neojs-builtins

Engine-independent Rust implementations of Neo built-in modules shared by the
V8 runtime (`neojs-core`) and the QuickJS runtime (`neojs-lite-core`).

The crate contains Rust logic and metadata only. It has no `deno_core`
dependency, no `rquickjs` dependency, and performs no I/O, so it can sit in both
dependency graphs without moving an engine across the separation boundary. Each
engine binds the same Rust functions with its own native function and module
objects; there is no ECMAScript implementation source in this crate.

```sh
cargo test -p neojs-builtins --all-features
cargo test -p neojs-builtins
cargo tree -p neojs-builtins -e normal
```

## What is shared

Each module contributes:

* a Rust implementation with total predicates and explicit case-mapping results;
* a `BuiltinModule` descriptor with the canonical specifier, the exact export
  names, and the Rust backend marker.

Each engine keeps its own binding. V8 creates native V8 function objects,
registers a native synthetic module with the deno_core seam, and installs the
same object in the registry. QuickJS creates native QuickJS functions and
exports them from a native QuickJS module. In both engines the registry property
and the module export are the same function object; the ES module namespace is a
distinct object.

## Module selection

Every shared module has its own Cargo feature. `chars` is the first.

* `neojs-core` has a positive `neo-chars` feature that enables the optional
  `neojs-builtins` dependency and its `chars` feature. A default V8 build
  enables `neo-chars`. `neo-slices` implies `neo-chars` because `CharSlice`
  captures the native `chars.toUpper` and `chars.toLower` functions. Disabling
  `neo-chars` also disables `slices` and removes the native registration, the
  registry entry, the loader entry, and the binding bytes.
* `neojs-lite-core` has the same positive `neo-chars` feature, enabled by its
  default feature set. `default-features = false` removes the dependency, the
  native object, the module declaration, the registry entry, and the
  introspection list entry.
* `neojs-builtins` itself has no default module features. A build without
  `chars` has no descriptor and no classifier code.

Cargo features are additive and unify across the dependency graph. Module
selection is compile-time surface selection, not a sandbox.

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

* **Category data is Unicode 16.0; case data follows the Rust compiler.**
  Classification uses the Unicode 16.0 General_Category tables from
  `unicode-general-category`. Case mapping uses Rust's `char::to_lowercase` and
  `char::to_uppercase`, whose Unicode release is the compiler's and is not
  guaranteed to match the category table. A compiler upgrade can therefore
  change a case mapping without changing classification. Both can differ from
  the previous JavaScript implementation's engine tables. The contract tests
  pin the intended one-code-point behavior and a Unicode 16 classification
  boundary. `isSpace` is intentionally Go's `unicode.IsSpace` contract, not
  JavaScript `\s`: U+FEFF is not whitespace.
* **Captured native functions.** The engine creates the function objects once,
  before user code runs. Replacing a JavaScript intrinsic afterwards does not
  change the module. Code that replaces a realm intrinsic before the runtime is
  constructed is not defended against.
* **One module instance per realm.** The registry object is frozen and cached.
  The ES module namespace is a distinct object whose named exports are the same
  function objects as the registry properties.
* **Rust signatures are internal.** The public contract is the JavaScript
  module behavior. The crate's exact Rust signatures are not a stable ABI.

## Cross-engine tests

`tests/fixtures/chars_contract.js` is run by both `neojs-core` and
`neojs-lite-core`. Each test replaces two tokens with its adapter's specifier and
registry key. The fixture covers Unicode boundaries, scalar and category edge
cases, expanding case mappings, error classes, captured intrinsics, and module
import identity. Engine-specific tests additionally assert that every export is
a native function (`Function.prototype.toString` contains `[native code]`).

## V8 native module seam

`deno_core` 0.413.0 has no public API for registering an already-built synthetic
module with explicit named exports. The workspace therefore pins a vendored copy
with one added method, `JsRuntime::register_native_module`, documented in
`vendor/deno_core/PATCH.md`. The method uses the crate's existing
`ModuleMap::new_synthetic_module`; it does not change loader, resolver, cache, or
module-graph behavior. The V8 loader serves only an empty deduplication sentinel
for `neo:chars`, and the module map reuses the native module.

## Artifact checks

The engine test suites include artifact scans. A scan reads its own test
executable and looks for the Rust `chars` error text, which only links when the
module is compiled in. A disabled build must not contain it; an enabled build
must. A control string from the always-present `Neo` surface proves the scan is
reading real bytes. The sequences are decoded from hex at runtime, so the test
source cannot satisfy its own scan.
