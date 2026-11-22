# Third-party notices for `neo:path`

The `path` block in `neojs-core/src/builtins/neo.js` and the re-exports in
`neojs-core/src/builtins/path.esm.js` adapt lexical path algorithms from the
NeoLiteJS project and from `@neotales/path` in the `js-libraries` repository.
No glob or file URL code was copied.

## NeoLiteJS `neo:path`

- Source: <https://github.com/neotales/neolitejs>
- File: `neojs-core/src/js/path.js`
- Revision inspected: `bafa9bc14cfbdbc50d64b3c68d63d1fceb2b8afa` (2026-11-21)
- File `neojs-core/src/js/path.LICENSE.md` SHA-256:
  `26607003517b277b2611beb8d66cf9d2c37e4b1064df7250f17dacede2cf8a2a`
- File `LICENSE` SHA-256:
  `37907d2ea9dad1c6081283a3b2032b3f4f4c5440358e068ab330ce0454d6ffc0`
- License: MIT
- Copyright (c) 2026 NeoTales contributors

NeoLiteJS `neo:path` is itself an adaptation of `@neotales/path`; the notices
for that upstream are reproduced below.

## `@neotales/path`

- Upstream: <https://github.com/neotales/js-libraries>
- Directory: `std/jsr/path`
- Revision: `c7e88644f45a78f65bf639c9d365bb9eff21bcfe` (2026-09-28)
- Last change touching this directory:
  `2656f91e43b183927d1fdc3c501e77ab4b1ccce4` (2026-09-27)
- License: MIT
- File `std/jsr/path/LICENSE.md` at that revision, SHA-256:
  `4a9edd22e4358ff7607cfd6e056e8640712b5e60100a5c339f842e02d3ad8acb`
- Checkout state: `git status` and `git diff` were clean for `std/jsr/path`
  when inspected, and the license file matches the recorded revision.

Adapted source files, algorithms only:

- `posix/_util.ts`, `posix/normalize.ts`, `posix/join.ts`, `posix/basename.ts`,
  `posix/dirname.ts`, `posix/extname.ts`, `posix/is_absolute.ts`,
  `posix/parse.ts`, `posix/format.ts`, `posix/resolve.ts`,
  `posix/constants.ts`
- `windows/_util.ts`, `windows/normalize.ts`, `windows/join.ts`,
  `windows/basename.ts`, `windows/dirname.ts`, `windows/extname.ts`,
  `windows/is_absolute.ts`, `windows/parse.ts`, `windows/format.ts`,
  `windows/resolve.ts`, `windows/to_namespaced_path.ts`,
  `windows/constants.ts`
- `_common/normalize_string.ts`, `_common/basename.ts`, `_common/dirname.ts`,
  `_common/format.ts`, `_common/strip_trailing_separators.ts`,
  `_common/assert_path.ts`, `_common/common.ts`
- `types.ts` (the `ParsedPath` shape)

NeoJS changed the adaptation as follows:

- Wrapped the implementation in a scope inside the `neo.js` IIFE and selected
  the host dialect from the private `opNeoPathIsWindows` op, not from a
  script-replaceable global.
- Removed the glob and file URL helpers, URL inputs, and the `PATH_PERMISSION`
  branch. NeoJS has no permission system and no `URL` global.
- Added `PathError` with an own `name` and `code`, and mapped a missing or
  non-UTF-8 working directory to `PATH_CWD` instead of a permission error.
- Added the cross-dialect cwd rule: a non-host dialect that needs a working
  directory reports `PATH_CWD_DIALECT`.

The upstream files carry these header lines:
`// Copyright 2018-2025 the Deno authors. MIT license.`,
`// Copyright 2018-2026 the Deno authors. MIT license.` (one file), and
`// Copyright the Browserify authors. MIT License.` The complete license texts
follow.

---

MIT License

Copyright (c) 2026 NeoTales contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

MIT License

Copyright 2022-2026 Neotales

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the “Software”), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

---

MIT License

Copyright 2018-2022 the Deno authors.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

MIT License

Copyright (c) 2013 James Halliday

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
