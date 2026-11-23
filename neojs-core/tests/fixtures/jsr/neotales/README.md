# `@neotales/chars` and `@neotales/slices` JSR fixtures

These are **unmodified** responses from the public JSR registry, captured once
over HTTPS without credentials. They are real registry documents and package
source, not synthesized shapes. No sanitization was needed: the captured bytes
contain only public package metadata, source, and license text. They are stored
here so the integration tests run offline and deterministically.

- Capture date: 2026-11-22 (UTC).
- Registry origin: `https://jsr.io`
- Versions: `@neotales/chars@0.0.0` and `@neotales/slices@0.0.0`. The fixtures
  pin these exact published version numbers; they do not use a dist-tag.
- Note: `https://jsr.io/@neotales/char/meta.json` returns HTTP 404. The package
  is named **chars**; there is no `char` package in that scope. The suggestion
  to test `@neotales/char` is therefore represented by `@neotales/chars`.
- `@neotales/slices@0.0.0` imports `jsr:@neotales/chars@^0.0.0/...`, which the
  resolver selects as `@neotales/chars@0.0.0`. That is the cross-package graph
  exercised by the tests.

The captured source is MIT-licensed; `LICENSE.md` is the package license text
from both packages (identical bytes).

### `LICENSE.md`

`https://jsr.io/@neotales/chars/0.0.0/LICENSE.md` (the same file is published by
`@neotales/slices@0.0.0`; the manifest records the same checksum)

```text
c8fa1e1ad104e8ef7fc3724f146cc135d13e68f28af88de317fd2c4f587a47f8
```

## Source URLs and SHA-256 of the captured bytes

Each hash is of the exact file in this directory tree.

### `chars/meta.json`

`https://jsr.io/@neotales/chars/meta.json`

```text
04468164a20fbc970311948befc021cb8224bd8cc5f39195d8c57c9c05327e5f
```

### `chars/0.0.0_meta.json`

`https://jsr.io/@neotales/chars/0.0.0_meta.json`

```text
30c45eeeb7eedd38ce50b3a1c77f6aaa8c14790d1bb2cbe8a2ed25c0b2141401
```

The document's `manifest` checksums match the files below. Its `moduleGraph2`
field is retained unmodified; neojs does not interpret it.

### `chars/0.0.0/...`

| Path | Source URL | SHA-256 |
| --- | --- | --- |
| `is_digit.ts` | `https://jsr.io/@neotales/chars/0.0.0/is_digit.ts` | `1ab94b9aa7c85e0b5b81df1e66fb7e448b421513837a8ec27424317e28720198` |
| `to_lower.ts` | `https://jsr.io/@neotales/chars/0.0.0/to_lower.ts` | `0eef5d883190433ce2c657cb0f0f13eb4ed727fa0b942d0989e6721d227fd537` |
| `to_upper.ts` | `https://jsr.io/@neotales/chars/0.0.0/to_upper.ts` | `4b7bd0b2149893c7cb12daafd88c08a01caef0b562ddc5728fd22478eab0fb8f` |
| `constants.ts` | `https://jsr.io/@neotales/chars/0.0.0/constants.ts` | `921fa387d81dbf4032451bb69467a462de746460c95c06c94d11760a92db8d65` |
| `tables/nd.ts` | `https://jsr.io/@neotales/chars/0.0.0/tables/nd.ts` | `425c0681e0f7b0ee6e37799a4bf7b468636b9eebedc7685451178ab7633edcd9` |
| `tables/latin1.ts` | `https://jsr.io/@neotales/chars/0.0.0/tables/latin1.ts` | `b8ea9a962f82d24d33ef9477e524794095bf0b149dfba760860b3746d7dcccf1` |
| `tables/case.ts` | `https://jsr.io/@neotales/chars/0.0.0/tables/case.ts` | `bab59116cc0428ccfaf504dab4e27fb9ff84422fa8e17a4f3ca7b6f7d103a9cd` |

`types.ts` is imported only with `import type` and is erased before loading, so
it is not part of the captured runtime closure.

### `slices/meta.json`

`https://jsr.io/@neotales/slices/meta.json`

```text
296e94dbdc1d458deb2a9526cc658018143dca4dad21f5007e7ad2680049d911
```

### `slices/0.0.0_meta.json`

`https://jsr.io/@neotales/slices/0.0.0_meta.json`

```text
e6c18743de4ba4e441ef272fc51e8b48afceab576e66a2f68bb9dda51ac108e6
```

### `slices/0.0.0/...`

| Path | Source URL | SHA-256 |
| --- | --- | --- |
| `capitalize.ts` | `https://jsr.io/@neotales/slices/0.0.0/capitalize.ts` | `e8e696d80a3aa72ec9de50b6fece9304f5826e429d014434f62493705e075ab4` |
| `utils.ts` | `https://jsr.io/@neotales/slices/0.0.0/utils.ts` | `69e4245d5f340292ece9ff952fdf117ec611c7a94a3c020e7c2f9b03cc809191` |

## Scope

The tests import `jsr:@neotales/chars@0.0.0/is-digit` and
`jsr:@neotales/slices@0.0.0/capitalize`. The files are ordinary ECMAScript after
TypeScript type erasure and use no Deno-only APIs. The fixtures do not claim
registry behavior beyond these documents, and no live network is used by the
test suite.
