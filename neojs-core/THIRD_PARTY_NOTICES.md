# Third-party notices

`neojs-core` contains code adapted from the NeoLiteJS project and uses the
`neotales-age` and `neotales-ssh` crates at runtime.

## NeoLiteJS `neo:crypto` and `neo:age`

Source: NeoLiteJS, `neojs-core/src/crypto.rs` and `neojs-core/src/age.rs`.

Copyright (c) 2026 NeoTales contributors

Licensed under the MIT License:

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

## NeoLiteJS `neo:assert`

Source: NeoLiteJS `neojs-core/src/js/assert.js`.

Revision `bafa9bc14cfbdbc50d64b3c68d63d1fceb2b8afa`; file SHA-256
`e0f032306c0b2179941978483a80d6b0d3bc2fec8e6cf09552854052cad7dfbb`.

`neojs-core/src/builtins/neo.js` adapts comparison, cycle tracking, value formatting,
and assertion-error structure from this MIT-licensed source for the `assert` built-in. The
adaptation renames the equality surface to `equals` and `strictEquals`, removes the NeoLiteJS
`guard` dependency in favour of local synchronous and Promise callback handling, adds
`AssertionError` with a neojs-specific `NEO_ASSERT` code, and combines a Map key and value into one
rollback checkpoint. The API is not Node or Deno compatible.

Copyright (c) 2026 NeoTales contributors

Licensed under the MIT License:

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

## NeoLiteJS `neo:test`

Source: NeoLiteJS `neojs-core/src/js/test.js`.

Revision `bafa9bc14cfbdbc50d64b3c68d63d1fceb2b8afa`; file SHA-256
`79a5fb09e85205c996a8f435a36e908a12af990ada833a8c6b4e1dd8100c6776`.

`neojs-core/src/builtins/neo.js` adapts the registration queue, failure
bookkeeping, report shape, and prototype-independent append helpers from this
MIT-licensed source for the `test` built-in. The adaptation adds asynchronous
callbacks with per-test timeouts, pending tests, unhandled-rejection capture,
top-level-await registration, a bounded event-loop settle check, and the host
report used by `neojs test`; it
removes the synchronous-only restriction and does not implement Node's
`node:test`. The API is not Node compatible beyond the `test(name, fn)` call
shape.

Copyright (c) 2026 NeoTales contributors

Licensed under the MIT License:

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

## PLAB `ShamirSecretSharing` and rust-libraries `shamir`

Source: PLAB `dn/lib/crypto/Crypto/src/ShamirSecretSharing.cs` at revision
`79faf48ab85d23fcdbf64e29924ac0338841cb58`, released under the MIT License,
Copyright (c) Neotales and x0craft; and rust-libraries
`crypto/crypto/src/shamir.rs`, released under the MIT License, Copyright (c)
2026 Neotales.

`neojs-core/src/crypto_shamir.rs` independently implements the same GF(256)
field, share encoding, and interpolation. It does not link, call, or vendor
either source. The field reduction polynomial `0x11b`, generator `0x03`, and
one-based share indexes are interoperability facts, not copied code. The
upstream MIT terms are reproduced here because the share format and algorithm
were adapted from those implementations.

MIT License

Copyright (c) Neotales and x0craft, and Copyright (c) 2026 Neotales

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

## `neotales-age`

Source: <https://github.com/neotales/rs-age>, crate `neotales-age` 0.0.0-alpha.1.

`neojs-core` links this crate to implement the age v1 format and X25519
encryption. `neojs-core/src/age.rs` adds the JavaScript-facing API, the
X25519-only and binary-only policy, the size bounds, and the error mapping; it
does not modify the crate.

Copyright (c) 2026 Neotales and x0craft

Licensed under the MIT License:

MIT License

Copyright (c) 2026 Neotales and x0craft

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

## `neotales-ssh`

Source: <https://github.com/neotales/rs-ssh>, crate `neotales-ssh`
0.0.0-alpha.1.

`neojs-core` links this crate to implement the SSH-2 wire codec, key exchange,
host-key verification, user authentication, protected transport, and session
channels used by the `ssh` built-in. `neojs-core/src/ssh.rs` adds the
JavaScript-facing `connect`/`exec`/`close` slice, the mandatory explicit
host-key or known-hosts policy, the in-memory credential handling, the size
bounds, the deadline handling, and the error mapping; it does not modify the
crate. The crate declares `rust-version = "1.100"`, the same minimum supported
Rust version as this workspace.

Copyright (c) 2026 [Neotales](https://github.com/neotales) and
[x0craft](https://github.com/x0craft)

Licensed under the MIT License:

MIT License

Copyright (c) 2026 Neotales and x0craft

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

## `sha2` and `hmac` (RustCrypto)

Source: <https://github.com/RustCrypto/hashes>, crate `sha2` 0.11.1, and
<https://github.com/RustCrypto/MACs>, crate `hmac` 0.13.0.

`neojs-core` links these crates directly for the SHA-256, SHA-384, SHA-512,
and HMAC implementation behind `neo:crypto`. They were already present in the
dependency graph through the `neotales-age` and `neotales-ssh` backends; the
hash surface reuses the same reviewed versions. `neojs-core/src/crypto_hash.rs`
adds the JavaScript-facing lifecycle, encoding, bound, typed-array, and
executor policy; it does not modify either crate.

Both crates are dual-licensed under the MIT License or the Apache License,
Version 2.0. `neojs-core` uses them under the MIT option. The copyright lines
from their MIT licence files are:

- `sha2`: Copyright (c) 2016-2026 The RustCrypto Project Developers;
  Copyright (c) 2016 Artyom Pavlov; Copyright (c) 2009-2013 Mozilla Foundation;
  Copyright (c) 2006-2009 Graydon Hoare.
- `hmac`: Copyright (c) 2017 Artyom Pavlov.

The MIT permission notice follows.

MIT License

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

## `pbkdf2` (RustCrypto)

Source: <https://github.com/RustCrypto/password-hashes>, crate `pbkdf2` 0.13.0.

`neojs-core` links this crate directly for the PBKDF2-HMAC-SHA-256, PBKDF2-HMAC-SHA-384, and
PBKDF2-HMAC-SHA-512 implementation behind `neo:crypto`. It was already present in the dependency
graph through `neotales-age`'s `scrypt` dependency; the password surface reuses the same reviewed
version with its `sha2` feature. `neojs-core/src/crypto_hash/password.rs` adds the JavaScript-facing
argument, bound, work-factor, executor, and error policy; it does not modify the crate.

The crate is dual-licensed under the MIT License or the Apache License, Version 2.0. `neojs-core`
uses it under the MIT option. The copyright lines from its MIT licence file are:

- Copyright (c) 2017 Artyom Pavlov
- Copyright (c) 2018-2023 The RustCrypto Project Developers

The MIT permission notice follows.

MIT License

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

## `bcrypt` and `blowfish`

Source: <https://github.com/Keats/rust-bcrypt>, crate `bcrypt` 0.19.3, and
<https://github.com/RustCrypto/block-ciphers>, crate `blowfish` 0.10.0.

`neojs-core` links `bcrypt` directly for bcrypt modular-crypt parsing and formatting, salt handling,
and constant-time verification. `bcrypt` uses the RustCrypto `blowfish` crate's bcrypt variant for
the key schedule and block encryption. `blowfish` was already present in the dependency graph
through `neotales-x509`; the password surface reuses the same reviewed version.
`neojs-core/src/crypto_hash/password.rs` adds the JavaScript-facing argument, bound, work-factor,
executor, and error policy; it does not modify either crate. `bcrypt` declares the same edition as
this workspace and builds with Rust 1.100.0.

`bcrypt` is licensed under the MIT License. `blowfish` is dual-licensed under the MIT License or the
Apache License, Version 2.0, and `neojs-core` uses it under the MIT option. The copyright lines from
their MIT licence files are:

- `bcrypt`: Copyright (c) 2020 Vincent Prouillet.
- `blowfish`: Copyright (c) 2016-2024 The RustCrypto Project Developers.

The MIT permission notice follows.

MIT License

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

## `aes-gcm`, `cbc`, `aes`, and `cipher` (RustCrypto)

Source: <https://github.com/RustCrypto/AEADs>, crate `aes-gcm` 0.10.3;
<https://github.com/RustCrypto/block-modes>, crate `cbc` 0.2.1;
<https://github.com/RustCrypto/block-ciphers>, crate `aes` 0.9.4; and
<https://github.com/RustCrypto/traits>, crate `cipher` 0.5.2.

`neojs-core` links `aes-gcm` for the NEO1 envelope and one-shot GCM, and links
`cbc` with `aes`/`cipher` for the stateful AES-256-CBC and AES-256-GCM surface.
The `cbc` crate was already present in the dependency graph through
`neotales-age`; the direct dependency reuses the reviewed 0.2.1 version. This
repository's `neojs-core/src/crypto.rs` and `neojs-core/src/crypto_cipher.rs`
add the JavaScript-facing envelope, lifecycle, size bounds, typed-array
validation, and error mapping; they do not modify the crates.

All four crates are dual-licensed under the MIT License or the Apache License,
Version 2.0. `neojs-core` uses them under the MIT option. The copyright lines
from their MIT licence files are:

- `aes-gcm`: Copyright (c) 2019 The RustCrypto Project Developers.
- `cbc`: Copyright (c) 2018-2022 RustCrypto Developers; Copyright (c) 2018
  Artyom Pavlov.
- `aes`: Copyright (c) 2018-2024 The RustCrypto Project Developers; Copyright
  (c) 2018 Artyom Pavlov.
- `cipher`: Copyright (c) 2016-2025 RustCrypto Developers.

The MIT permission notice follows.

MIT License

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

## NeoLiteJS `neo:path` and `@neotales/path`

Source: NeoLiteJS `neojs-core/src/js/path.js` and `path.LICENSE.md`, and the
`@neotales/path` module in `neotales/js-libraries`, directory `std/jsr/path`.

`neojs-core/src/builtins/neo.js` adapts lexical path algorithms from these
MIT-licensed sources for the `path` built-in. The adaptation removes the glob
and file URL helpers, URL inputs, and the NeoLiteJS `PATH_PERMISSION` branch;
it adds the two-namespace host-selection shape, argument validation, the NUL
rule, `PathError`, and the process-cwd policy. The exact upstream revisions,
adapted-file list, and complete license blocks are in
`neojs-core/src/builtins/path.LICENSE.md`.

- NeoLiteJS revision `bafa9bc14cfbdbc50d64b3c68d63d1fceb2b8afa`; file
  `neojs-core/src/js/path.LICENSE.md` SHA-256
  `26607003517b277b2611beb8d66cf9d2c37e4b1064df7250f17dacede2cf8a2a`.
- `@neotales/path` revision `c7e88644f45a78f65bf639c9d365bb9eff21bcfe`;
  LICENSE.md SHA-256
  `4a9edd22e4358ff7607cfd6e056e8640712b5e60100a5c339f842e02d3ad8acb`.

Copyright (c) 2026 NeoTales contributors

Copyright 2022-2026 Neotales

Copyright 2018-2022 the Deno authors.

Copyright (c) 2013 James Halliday

Licensed under the MIT License:

MIT License

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
