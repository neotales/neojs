---
title: vael:strings, vael:chars & vael:slices
description: Unicode 16.0 case-folded string matching, total Unicode scalar predicates, and zero-copy live Slice and CharSlice views.
---

`vael:strings`, `vael:chars`, and `vael:slices` provide deterministic text processing and zero-copy live array/typed-array views shared identically across V8 and QuickJS.

:::note[Availability]
All three modules (`vael:strings`, `vael:chars`, and `vael:slices`) are available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

## `vael:strings`

`vael:strings` implements locale-independent **Unicode 16.0 simple case folding** (Go `strings.EqualFold` semantics over UTF-16 code units) and code-point trimming. Lone surrogates fold to themselves, never match half of a surrogate pair, and are preserved in trimmed interiors.

| Export | Signature | Description |
| --- | --- | --- |
| `equalFold` | `(text: string, other: string) => boolean` | Case-folded equality. |
| `startsWithFold` | `(text: string, prefix: string) => boolean` | Case-folded prefix check. |
| `endsWithFold` | `(text: string, suffix: string) => boolean` | Case-folded suffix check. |
| `indexOfFold` | `(text: string, search: string, fromIndex?: number) => number` | First UTF-16 code-unit index at or after `fromIndex`, or `-1`. |
| `lastIndexOfFold` | `(text: string, search: string, fromIndex?: number) => number` | Last UTF-16 code-unit index at or before `fromIndex`, or `-1`. |
| `trim` | `(text: string, chars?: string) => string` | Trim leading and trailing code points in `chars` (defaults to ECMAScript whitespace). |
| `trimStart` | `(text: string, chars?: string) => string` | Trim leading code points in `chars` (or whitespace). |
| `trimEnd` | `(text: string, chars?: string) => string` | Trim trailing code points in `chars` (or whitespace). |

Non-string arguments throw `TypeError`; `NaN` or fractional `fromIndex` values throw `RangeError` (`Infinity` and `-Infinity` clamp to string bounds).

## `vael:chars`

`vael:chars` provides 12 **total predicates** accepting `unknown` (returning `false` for non-numbers, `NaN`, infinities, fractions, surrogates `0xD800..=0xDFFF`, and out-of-range numbers) and 2 single-code-point case mappers.

| Group | Exports | Semantics |
| --- | --- | --- |
| Scalar ranges | `isChar(v)`, `isAscii(v)`, `isLatin1(v)` | `0..=0x10FFFF` (non-surrogate), `0..=0x7F`, `0..=0xFF`. |
| Unicode categories | `isControl(v)`, `isDigit(v)`, `isLetter(v)`, `isLetterOrDigit(v)`, `isLower(v)`, `isUpper(v)`, `isPunc(v)`, `isSymbol(v)` | Categories `Cc`, `Nd`, `L`, `L \| Nd`, `Ll`, `Lu`, `P`, `S` (incl. emoji). |
| Whitespace | `isSpace(v)` | Go `unicode.IsSpace` (`\t`, `\n`, `\v`, `\f`, `\r`, space, `U+0085`, `Zs`/`Zl`/`Zp`; `U+FEFF` & `U+200B` are `false`). |
| Case mapping | `toLower(value: number): number`, `toUpper(value: number): number` | Single-code-point mapping; multi-code-point expansions (e.g. `U+00DF` `ß`) remain unchanged. Throws `TypeError` / `RangeError` on invalid input. |

## `vael:slices`

`vael:slices` provides fixed `[start, end)` **zero-copy live views** over `Array`s, genuine `TypedArray`s (excluding `DataView` and typed-array proxies), and `Uint32Array` Unicode code-point buffers. Construction, nested `.slice()`, `.at()`, `.set()`, `.indexOf()`, `.includes()`, and iteration never copy; `.copy()`, `.toArray()`, and `.toString()` create explicit snapshots.

| Class / Factory | Target | Members |
| --- | --- | --- |
| `Slice` / `slice(target, start?, end?)` | `E[] \| TypedArray` | `length`, `isEmpty`, `at(i)`, `set(i, v)`, `slice(s?, e?)`, `copy()`, `indexOf(v)`, `includes(v)`, `[Symbol.iterator]()` |
| `ReadOnlySlice` / `readonlySlice(target, start?, end?)` | `E[] \| TypedArray` | Same as `Slice` without `set()` (observes mutations on underlying target). |
| `CharSlice` / `charSlice(buffer, start?, end?)` | `Uint32Array` | `static fromString(text)`, `length`, `isEmpty`, `at(i)`, `set(i, cp)`, `slice(s?, e?)`, `toUpper()`, `toLower()`, `toString()`, `toArray()`, `[Symbol.iterator]()` |
| `ReadOnlyCharSlice` / `readonlyCharSlice(buffer, start?, end?)` | `Uint32Array` | Same as `CharSlice` without `set()`, `toUpper()`, or `toLower()`. |

## Example

```ts
import { isLetter, toUpper } from "vael:chars";
import { CharSlice, slice } from "vael:slices";
import { equalFold, trim } from "vael:strings";

console.log(equalFold("Straße", "STRASSE")); // false (simple folding)
console.log(equalFold("VaelJS", "vaeljs"));   // true
console.log(trim("///path///", "/"));        // "path"
console.log(isLetter(0x41), String.fromCodePoint(toUpper(0x61)));

const buf = new Uint8Array([10, 20, 30, 40]);
const view = slice(buf, 1, 3);
view.set(0, 99);
console.log(buf[1], view.copy()); // 99, Uint8Array [99, 30]

const chars = CharSlice.fromString("hello");
console.log(chars.slice(0, 2).toUpper().toString()); // "HE"
```
