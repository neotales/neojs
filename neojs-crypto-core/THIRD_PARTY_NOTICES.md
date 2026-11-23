# Third-party notices

`neojs-crypto-core` contains the engine-independent envelope, associated-data
construction, size policy, and core seal/open logic adapted from the
`neo:crypto` module in the NeoLiteJS project (`neojs-core/src/crypto.rs`). That
source is MIT licensed.

The crate's runtime dependencies `aes-gcm` (RustCrypto) and `getrandom` are
licensed under MIT OR Apache-2.0; this workspace uses the MIT terms. See the
lockfile-pinned crate metadata for their full licence texts.

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
