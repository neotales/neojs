# Third-party notices

`vaeljs-crypto-core` contains the engine-independent envelope, associated-data
construction, size policy, and core seal/open logic adapted from the
`vael:crypto` module in the NeoLiteJS project (`vaeljs-core/src/crypto.rs`),
and the GF(256) Shamir field and share encoding adapted from PLAB
`ShamirSecretSharing` and `neotales-crypto`. It also uses RustCrypto crates,
`bcrypt`, `getrandom`, `zeroize`, and `thiserror`.

## NeoLiteJS `neo:crypto`

Source: NeoLiteJS, `neojs-core/src/crypto.rs`.

Copyright (c) 2026 NeoTales contributors

Licensed under the MIT License:

```text
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
```

## PLAB `ShamirSecretSharing` and rust-libraries `shamir`

Source: PLAB `dn/lib/crypto/Crypto/src/ShamirSecretSharing.cs` at revision
`79faf48ab85d23fcdbf64e29924ac0338841cb58`, released under the MIT License,
Copyright (c) Neotales and x0craft; and rust-libraries
`crypto/crypto/src/shamir.rs`, released under the MIT License, Copyright (c)
2026 Neotales.

`vaeljs-crypto-core::shamir` independently implements the same GF(256) field,
share encoding, and interpolation. It does not link, call, or vendor either
source. The field reduction polynomial `0x11b`, generator `0x03`, and one-based
share indexes are interoperability facts, not copied code. The upstream MIT terms
are reproduced here because the share format and algorithm were adapted from
those implementations.

```text
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
```

## RustCrypto (`aes-gcm`, `cbc`, `aes`, `cipher`, `sha2`, `hmac`, `pbkdf2`) and `bcrypt` / `blowfish`

The crate's runtime dependencies `aes-gcm`, `cbc`, `aes`, `cipher`, `sha2`,
`hmac`, `pbkdf2`, `blowfish`, `getrandom`, `zeroize`, and `thiserror` are
dual-licensed under MIT OR Apache-2.0; `bcrypt` is MIT licensed. This workspace
uses the MIT terms. See the lockfile-pinned crate metadata for their full licence
texts.
