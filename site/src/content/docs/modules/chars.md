---
title: chars
description: Unicode code-point classification and simple case mapping, implemented as plain ECMAScript.
---

`chars` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `chars`
or `neo:chars`; the registry form is `Neo.getBuiltInModule("chars")`.

It is plain ECMAScript and calls no host op. There is no default export and there are no string
overloads or `...At` helpers: pass a code point, usually `text.codePointAt(0)`.

## Exports

| Export | True for |
| --- | --- |
| `isChar` | A Unicode scalar value: an integer from 0 through `0x10FFFF`, excluding surrogates |
| `isAscii` | A scalar value from 0 through `0x7F` |
| `isLatin1` | A scalar value from 0 through `0xFF` |
| `isControl` | Category `Cc` |
| `isDigit` | Category `Nd` (decimal digits, not other numerals) |
| `isLetter` | Category `L` |
| `isLetterOrDigit` | Category `L` or `Nd` |
| `isLower` | Category `Ll` |
| `isUpper` | Category `Lu` |
| `isPunc` | Category `P` |
| `isSpace` | Whitespace, described below |
| `isSymbol` | Category `S`, including emoji |
| `toLower` | The lowercase code point, when the mapping is one code point |
| `toUpper` | The uppercase code point, when the mapping is one code point |

```js
import { isLetter, isDigit, toUpper } from "chars";

console.log(isLetter("λ".codePointAt(0))); // true
console.log(isDigit("٣".codePointAt(0))); // true
console.log(toUpper("a".codePointAt(0))); // 65
```

## Predicates

Every predicate takes a Unicode code point as a number. It returns `false` for anything that is not
a Unicode scalar value: a non-number, a non-integer, `NaN`, a value outside `0` through `0x10FFFF`,
or a surrogate (`0xD800` through `0xDFFF`). Predicates never throw.

`isSpace` follows Go's `unicode.IsSpace`: tab, line feed, vertical tab, form feed, carriage return,
space, `U+0085`, and categories `Zs`, `Zl`, and `Zp`. Unlike JavaScript's `\s`, it includes `U+0085`
and `U+00A0` but excludes `U+FEFF`.

## Case mapping

`toLower` and `toUpper` throw `TypeError` for a non-number and `RangeError` for a non-scalar value.
They return the input unchanged when the host's full mapping expands to more than one code point,
so `ß` stays `ß` rather than becoming `SS`.

## Unicode version

Classification uses the Unicode property tables compiled into V8, so the Unicode version follows
the V8 build. This is an independent implementation informed by `@neotales/chars` and the QuickJS
`neo:chars` module; it does not port their lookup tables and does not claim compatibility with
either API.

## Not implemented

The `...At` string-index helpers, the `...Unsafe` variants, `equalFold`, `simpleFold`, and character
code constants are not part of this slice. A future built-in name is not promised: an unknown
registry name returns `undefined` and an import of an unknown module is an error.
