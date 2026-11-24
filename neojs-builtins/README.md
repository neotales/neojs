# neojs-builtins

Engine-independent implementations of Neo built-in modules shared by the V8
runtime (`neojs-core`) and the QuickJS runtime (`neojs-lite-core`).

The crate contains Rust logic, shared ECMAScript factory/wrapper sources, and
metadata only. It has no `deno_core` dependency, no `rquickjs` dependency, and
performs no I/O, so it can sit in both dependency graphs without moving an
engine across the separation boundary.

```sh
cargo test -p neojs-builtins --all-features
cargo test -p neojs-builtins
cargo tree -p neojs-builtins --all-features -e normal
```

## What is shared

Each module contributes a `BuiltinModule` descriptor with its canonical `neo:`
specifier, exact export names, and `Backend` (`Backend::Rust` or
`Backend::Ecmascript`):

* **`Backend::Rust` (`chars`, `os`, `strings`, `uuid`)**: pure Rust
  implementations with total predicates, target constants, or explicit error
  results. V8 binds native V8 function objects or host ops; QuickJS exports
  native QuickJS values and functions from native modules.
* **`Backend::Ecmascript` (`assert`, `test`)**: shared ECMAScript factory
  functions (`assert.js`, `test.js`) and `.esm.js` re-export wrappers
  parameterized by engine-supplied host hooks (`isNativePromise`,
  `observePromise`, `print`, rejection tracking, job/event-loop checks, and
  optional timers).

In both engines, the registry entry and the ES module exports share function and
class identity while the ES module namespace remains a distinct object.

## Module selection

Every shared module has its own Cargo feature: `assert`, `chars`, `os`,
`strings`, `test`, and `uuid` (`uuid` registers `neo:uuid`, `neo:uuid/v4`, and
`neo:uuid/v7`).

* `neojs-core` enables `assert`, `os`, `test`, and `uuid` unconditionally and
  `chars` and `strings` in its default feature set. `neo-slices` implies
  `neo-chars` because `CharSlice` captures the native `chars.toUpper` and
  `chars.toLower` functions.
* `neojs-lite-core` enables `neo-assert`, `neo-chars`, `neo-os`, `neo-strings`,
  `neo-test`, and `neo-uuid` in its default feature set.
  `default-features = false` removes the `neojs-builtins` dependency (and
  transitive `getrandom`/`uuid`), the registry entries, the loader entries, and
  the introspection list entries.
* `neojs-builtins` itself has no default module features. A build without a
  module has no descriptor and no implementation code for it.

Cargo features are additive and unify across the dependency graph. Module
selection is compile-time surface selection, not a sandbox.

## `neo:assert` and `neo:test`

* `neo:assert` (`assert` feature): callable `assert` / `ok`, `equals`,
  `notEquals`, `strictEquals`, `notStrictEquals`, `fail`, `throws`,
  `doesNotThrow`, `rejects`, `doesNotReject`, and `AssertionError` (`code:
  "NEO_ASSERT"`). Native Promises are checked and observed through engine hooks
  without consulting user `.then` or `Symbol.species`.
* `neo:test` (`test` feature): `test`, `run`, `MAX_TESTS`, and
  `TestFailureError` (`code: "NEO_TEST_FAILURES"`). When host timers are
  unavailable (QuickJS), `test(..., { timeout })` is rejected explicitly with
  `NEO_TEST_NO_TIMER`, and `run({ settle: true })` drains the bounded job queue.

The shared JavaScript contract suite in `tests/js/shared/*.test.js` runs under
both engines:

```sh
cargo run -p neojs -- test 'tests/js/shared/*.test.js'
cargo run -p neojs-lite -- test 'tests/js/shared/*.test.js'
```

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

## `neo:os`

The canonical specifier and registry key is `neo:os`.

| Export | Contract |
| --- | --- |
| `platform` | `std::env::consts::OS` for the compilation target (e.g. `"linux"`, `"macos"`, `"windows"`). |
| `arch` | `std::env::consts::ARCH` for the compilation target (e.g. `"x86_64"`, `"aarch64"`). |

Uses Rust's target vocabulary rather than Node's (`macos`, not `darwin`;
`aarch64`, not `arm64`). Performs no host probe, reads no environment variable
or file, and installs no global.

## `neo:uuid`

Canonical specifiers and registry keys: `neo:uuid` (exports `v4`, `v7`),
`neo:uuid/v4` (named `v4` and `default`), and `neo:uuid/v7` (named `v7` and
`default`). Both engines share one function object per version across all
specifiers.

| Export | Contract |
| --- | --- |
| `v4` | Synchronous RFC 9562 version 4 UUID (36-char lowercase hyphenated text) from OS CSPRNG via fallible `getrandom::fill` + `uuid::Builder`. |
| `v7` | Synchronous RFC 9562 version 7 UUID (48-bit Unix ms timestamp + 74 random bits from OS CSPRNG). Refuses times outside `0..2^48` ms with `NEO_UUID_CLOCK`. |

Both reject any arguments with `TypeError`, have no insecure fallback, never
panic on CSPRNG failure (returning `Error` with `code: "NEO_UUID_RANDOM"` and a
fixed non-secret `cause`), and promise no monotonicity.

## `neo:strings`

The canonical specifier and registry key is `neo:strings`.

| Export | Contract |
| --- | --- |
| `equalFold` | Compare two strings under simple case folding. |
| `startsWithFold` | True when the text begins with the prefix under folding. |
| `endsWithFold` | True when the text ends with the suffix under folding. |
| `indexOfFold` | First folded occurrence as a UTF-16 code unit index, or `-1`. |
| `lastIndexOfFold` | Last folded occurrence at or before an optional limit, or `-1`. |
| `trim` | Remove leading and trailing code points in an optional set; with no set, exactly `String.prototype.trim`. |
| `trimStart` | Remove leading code points; with no set, exactly `String.prototype.trimStart`. |
| `trimEnd` | Remove trailing code points; with no set, exactly `String.prototype.trimEnd`. |

The functions operate on UTF-16 code units. A lone surrogate is compared as
itself and never matches half of a surrogate pair; indices are code unit
offsets, not code point offsets. Folding is Unicode 16.0 simple case folding
from a generated table, so `"K"` equals U+212A KELVIN SIGN, `"s"` equals
U+017F LONG S, `"I"` does not equal U+0131 DOTLESS I, and `"ss"` does not equal
`"ß"`. Non-string arguments throw `TypeError`; a non-integer index throws
`RangeError`, while `Infinity` and `-Infinity` clamp. `indexOfFold` and
`lastIndexOfFold` stream through a fixed 1024-code-unit engine window instead
of copying the whole text; a small `fromIndex` reads only the prefix the scan
needs. The binding bounds copied units and retained window memory; it does not
claim a time bound for V8's internal rope traversal inside `write_v2`. The
module calls no host op and adds no capability.

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
* **Folding is pinned to Unicode 16.0 while `chars` case mapping follows the
  compiler.** The `strings` table is fixed at Unicode 16.0, so both engines fold
  the same input the same way. `chars` case mapping still uses the compiler's
  `char` tables. A compiler upgrade can change a `chars` mapping without
  changing `strings` folding.
* **Captured native functions.** The engine creates the function objects once,
  before user code runs. Replacing a JavaScript intrinsic afterwards does not
  change the module. Code that replaces a realm intrinsic before the runtime is
  constructed is not defended against.
* **One module instance per realm.** The registry object is frozen and cached.
  The ES module namespace is a distinct object whose named exports are the same
  function objects as the registry properties.
* **Rust signatures are internal.** The public contract is the JavaScript
  module behavior. The crate's exact Rust signatures are not a stable ABI.

## Case-folding data

`neo:strings` folds with a generated table from the exact official Unicode 16.0
`CaseFolding.txt` artifact. The source URL, version header, SHA-256, selection
rules, and Unicode License V3 notice are in
[`data/unicode/README.md`](data/unicode/README.md) and
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

```sh
python3 scripts/generate_case_fold.py
python3 scripts/generate_case_fold.py --check
```

The generator refuses a mismatched hash or version header. A unit test
independently hashes the pinned artifact and compares every Unicode code point
and every lone surrogate unit against `simple_fold`, without using the
generator.

## Cross-engine tests

`tests/fixtures/chars_contract.js` and `tests/fixtures/strings_contract.js` are
run by both `neojs-core` and `neojs-lite-core`. Each test replaces two tokens with its adapter's specifier and
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
for the native modules, and the module map reuses the native module.

## Artifact checks

The engine test suites include artifact scans. The V8 scan looks for the Rust
`chars` and `strings` error texts, which only link when the module is compiled
in, and for distinctive byte sequences from the still-JavaScript `slices`
implementation. The QuickJS scan looks for the Rust `chars` error text. A disabled build must not
contain the corresponding sequences, and an enabled build must. A control string
from the always-present `Neo` surface proves the scan is reading real bytes. The
sequences are decoded from hex at runtime, so the test source cannot satisfy its
own scan.
