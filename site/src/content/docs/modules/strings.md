---
title: strings
description: Case-folded search and code-point-set trimming, implemented as plain ECMAScript.
---

`strings` is available in every V8 runtime and is not available in NeoJS Lite. Import it as
`strings` or `neo:strings`; the registry form is `Neo.getBuiltInModule("strings")`.

It is plain ECMAScript: it calls no host op and adds no capability beyond the strings it is given.
There is no default export.

## Exports

| Export | Purpose |
| --- | --- |
| `equalFold` | Case-folded equality |
| `startsWithFold`, `endsWithFold` | Case-folded prefix and suffix tests |
| `indexOfFold`, `lastIndexOfFold` | Case-folded search returning UTF-16 code unit indices |
| `trim`, `trimStart`, `trimEnd` | Trim a caller-supplied set of Unicode code points |

```js
import { equalFold, indexOfFold, lastIndexOfFold, trim } from "strings";

console.log(equalFold("Hello WÖrLD", "hello wörld")); // true
console.log(indexOfFold("😀a", "A")); // 2 (UTF-16 code units)
console.log(lastIndexOfFold("aaa", "aa")); // 1 (overlapping)
console.log(trim("file.txt...", ".")); // "file.txt"
```

## Folding

Folded matching uses ECMAScript's simple case folding, decided per code point by V8's own RegExp
`/iu` canonicalization. It is locale-independent and is not a `toLowerCase()` comparison:

- `equalFold("K", "\u212A")` is `true` (Kelvin sign).
- `equalFold("s", "\u017F")` is `true` (long s).
- `equalFold("I", "\u0131")` is `false` (dotless i).
- `equalFold("ss", "ß")` is `false` (that is full folding).

Indices are UTF-16 code unit offsets, like `String.prototype.indexOf`, not code point offsets.
`indexOfFold` and `lastIndexOfFold` find overlapping occurrences, and an empty search returns the
clamped start index or the string length. A lone surrogate is compared as itself and never matches
half of a surrogate pair.

Searching is linear in the text plus the search string and uses O(search) extra memory: a repeated
prefix cannot make it quadratic. `lastIndexOfFold` also stops as soon as no match can begin at or
before `fromIndex`, so a small limit does not scan a long tail.

Argument values are not coerced: a non-string throws `TypeError`, and a non-integer `fromIndex`
throws `RangeError` (apart from `Infinity`, which clamps to the length).

## Trimming

With no `chars`, `trim`, `trimStart`, and `trimEnd` are exactly `String.prototype.trim`,
`trimStart`, and `trimEnd`, so they use JavaScript's whitespace set, which includes `U+FEFF` and
excludes `U+0085`. This is deliberately not [`neo:chars`](/modules/chars/)' `isSpace`, which follows
Go's `unicode.IsSpace`.

With `chars`, the argument is a set of Unicode code points, not a prefix or suffix. Every leading or
trailing code point in the set is removed:

```js
trim("##a##", "#"); // "a"
trim("file.txt...", "."); // "file.txt"
trim("...file", "."); // "file"
```

An empty `chars` string removes nothing.

## Scope

Folding and trimming are independent implementations informed by `@neotales/strings` and
`@neotales/slices`. They do not port those packages' tables and do not claim compatibility with
their code point indexing or their `Uint8Array`/`Uint16Array` buffer forms.
