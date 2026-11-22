//! Synchronous byte streams and process standard output for `neo:stdio`.
//!
//! The JavaScript module object is installed by [`crate::builtins`] and is
//! exposed as `Neo.getBuiltInModule("stdio")`, `import ... from "stdio"`, and
//! `import ... from "neo:stdio"`. This file registers the private host ops it
//! calls: one validates and copies bytes or UTF-8 text, one validates a read
//! target, one copies a byte range out of a private snapshot, and one writes to
//! the process's own standard output or standard error.
//!
//! # Scope
//!
//! The public surface is a memory-backed `Readable` over a bounded snapshot of
//! a `Uint8Array`, a memory-backed `Writable` over a bounded growable buffer,
//! and two shared `Writable` instances for the current process's stdout and
//! stderr. It is **not** Node's `stream` module, Node's `process`, or the WHATWG
//! Streams API. There are no events, listeners, promises, `pipe`, `on`,
//! backpressure, `Buffer` values, or encodings other than UTF-8 for string
//! arguments. Standard input, files, the working directory, the environment,
//! child processes, and the network are deliberately absent: this module adds
//! no filesystem or process authority. Writing to stdout and stderr uses the
//! same two descriptors the installed `console` already writes to, so it grants
//! no new capability. This is not a sandbox or an isolation boundary.
//!
//! The API shape, the bounded in-memory reader/writer, and the code-based error
//! reporting are informed by the MIT-licensed `neo:io` and `neo:process`
//! modules in the NeoLiteJS project. No source code is copied from them, and no
//! Node, Web, or NeoLiteJS compatibility is claimed.
//!
//! # Synchronous contract
//!
//! Every operation is synchronous and runs on the thread that owns the isolate.
//! A memory stream copies at most [`MAX_BYTES`] and never touches the operating
//! system. A host `writeSync` blocks until every byte has been accepted by the
//! operating system or an error occurs; it does not yield to the event loop,
//! has no timeout, and cannot be cancelled. If the consumer of the descriptor
//! stops reading, the call can block indefinitely. There is no asynchronous or
//! callback form in this slice, and none is faked: a future asynchronous surface
//! will be additive, `Promise`-based, and will perform its work on a real
//! asynchronous path rather than wrapping a blocking call in a resolved
//! `Promise`.
//!
//! # Reads, writes, and partial results
//!
//! `Readable.readSync(target)` copies at most `target.byteLength` bytes and
//! returns the number copied. It can copy fewer bytes than the target holds, so
//! a caller that wants all bytes must loop until it returns `0`. Exactly `0`
//! means end of file; end of file is not an exception. An empty target is an
//! argument error rather than an ambiguous zero. Closing a reader makes further
//! reads throw `NEO_STDIO_CLOSED`.
//!
//! `Writable.writeSync(chunk)` accepts a `Uint8Array` or a string encoded as
//! UTF-8 and returns the number of bytes accepted. A memory writer is
//! all-or-nothing: it appends the entire chunk and returns its byte length, or
//! it throws without changing the stream; it never returns a short count. A host
//! writer performs one blocking write and returns the byte length on success. A
//! host write that throws may already have put some bytes on the descriptor
//! because a system write can be partially completed before failing; there is
//! no rollback and no retry. The count is never short, so a successful call
//! needs no retry loop. Bytes written through this module and bytes written by
//! `console` share the same descriptor and are ordered by the calls that ran.
//!
//! `Writable.end()` retires the stream for writing but keeps memory-backed
//! content readable through `toUint8Array()`; on a host writer it does not close
//! the process descriptor. `Writable.close()` is terminal: it implies `end()`
//! and releases memory-backed content. On a host writer it only retires this
//! wrapper; the process descriptor stays open. Both are idempotent.
//!
//! # Ownership and bounds
//!
//! `Readable` copies the current byte range of its input view at construction,
//! so mutating, detaching, or resizing the caller's buffer afterwards does not
//! change the reader. `Writable` owns a copy of every appended chunk.
//! `toUint8Array()` returns a fresh `Uint8Array` owned by the caller; changing
//! it does not change the writer. `close()` never detaches a caller's buffer.
//!
//! [`MAX_BYTES`] bounds a reader's snapshot, one `writeSync` chunk, and a memory
//! writer's retained content. A memory writer is implemented as one coalesced
//! buffer whose capacity never exceeds [`MAX_BYTES`], rather than a chunk or
//! record per write; an empty write retains nothing. Growing the buffer
//! may briefly hold the old and new buffer at once. A refused copy or append is
//! reported before any state changes. The limit is per stream: this slice has no
//! aggregate native budget, and a `Uint8Array` returned by `toUint8Array()` is
//! JavaScript-owned and outside it. A string argument is encoded and measured in
//! UTF-8 bytes; the reported count is bytes, not UTF-16 code units. The
//! JavaScript heap, V8, and the Rust allocator allocate too, so recovery from
//! out-of-memory is not guaranteed.
//!
//! # Views and errors
//!
//! Inputs are checked through V8's internal typed-array brand and captured
//! intrinsics, so a forged, proxied, or detached view is rejected rather than
//! read as empty. A detached view reports `NEO_STDIO_DETACHED`. Offset views are
//! copied from their own byte range. Shared and resizable backing buffers are
//! accepted because the copy runs without executing JavaScript.
//!
//! Range copies used by `readSync` and `toUint8Array` are made with V8's native
//! view constructor and `copy_contents`, never with
//! `%TypedArray%.prototype.subarray`. Replacing `Uint8Array[Symbol.species]`,
//! `Uint8Array.prototype.constructor`, or a typed-array prototype method
//! therefore cannot run JavaScript, receive the private snapshot buffer, or
//! re-enter the module during a read. The wrapper's only typed-array writes use
//! the captured intrinsic `set`, which does not consult species.
//!
//! A value of the wrong type throws `TypeError`. Other failures are an ordinary
//! `Error` with `name = "StdioError"` and one stable own `code`:
//!
//! * `NEO_STDIO_ARGUMENT` — an empty read target or an invalid internal
//!   argument.
//! * `NEO_STDIO_DETACHED` — the view's backing store is detached.
//! * `NEO_STDIO_LIMIT` — the data would exceed [`MAX_BYTES`] or memory
//!   reservation failed.
//! * `NEO_STDIO_CLOSED` — the stream has been closed.
//! * `NEO_STDIO_ENDED` — the writer has ended.
//! * `NEO_STDIO_NOT_MEMORY` — `toUint8Array` was called on a host writer.
//! * `NEO_STDIO_WRITE` — the operating-system write failed.
//!
//! Messages are fixed except for the operating-system error text in
//! `NEO_STDIO_WRITE`; they never include the rejected bytes. The host ops return
//! a null-prototype result object instead of throwing, and the JavaScript
//! wrapper turns it into the documented exception.

use std::io::ErrorKind;
use std::io::Write;

use deno_core::op2;
use deno_core::v8;

/// Largest number of bytes one memory-backed stream retains, and the largest
/// single chunk accepted by `writeSync`.
///
/// The JavaScript module exports the same value as `MAX_BYTES`. The tests
/// exercise the boundary, so the two constants cannot drift silently.
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;

/// The stream selector passed by the JavaScript wrapper for standard output.
const STDOUT: usize = 1;
/// The stream selector passed by the JavaScript wrapper for standard error.
const STDERR: usize = 2;

/// A documented failure, carried to the JavaScript wrapper as data.
#[derive(Debug)]
struct StdioError {
    code: ErrorCode,
    message: String,
}

impl StdioError {
    fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.message().to_string(),
        }
    }

    /// A write failure names the stream and the operating-system error, never
    /// the bytes that were being written.
    fn write(stream: Stream, source: &std::io::Error) -> Self {
        Self {
            code: ErrorCode::Write,
            message: format!("cannot write to {}: {source}", stream.name()),
        }
    }
}

/// Stable failure kinds produced by the host side.
///
/// The JavaScript-only codes (`NEO_STDIO_CLOSED`, `NEO_STDIO_ENDED`, and
/// `NEO_STDIO_NOT_MEMORY`) are constructed in `neo.js`; they never come from an
/// op because the wrapper checks them before calling one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorCode {
    /// The value is not an accepted JavaScript type, reported as `TypeError`.
    Type,
    Detached,
    Limit,
    Argument,
    Write,
}

impl ErrorCode {
    fn js_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            _ => "StdioError",
        }
    }

    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Type => None,
            Self::Detached => Some("NEO_STDIO_DETACHED"),
            Self::Limit => Some("NEO_STDIO_LIMIT"),
            Self::Argument => Some("NEO_STDIO_ARGUMENT"),
            Self::Write => Some("NEO_STDIO_WRITE"),
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Type => "argument must be an attached Uint8Array or a string",
            Self::Detached => "the Uint8Array buffer is detached",
            Self::Limit => "byte data exceeds the stdio size limit",
            Self::Argument => "invalid stdio argument",
            Self::Write => "cannot write to the standard stream",
        }
    }
}

/// Which process descriptor a host write targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Self::Stdout => "standard output",
            Self::Stderr => "standard error",
        }
    }

    /// Reads the selector without consulting JavaScript hooks.
    fn from_value<'s>(
        scope: &v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Result<Self, StdioError> {
        match integer(scope, value) {
            Some(STDOUT) => Ok(Self::Stdout),
            Some(STDERR) => Ok(Self::Stderr),
            _ => Err(StdioError::new(ErrorCode::Argument)),
        }
    }

    /// Writes every byte and flushes, retrying interrupted and would-block
    /// writes as `deno_core`'s own print op does.
    ///
    /// The descriptor is the process's, not a per-runtime duplicate. The loop
    /// can spin while a nonblocking descriptor stays full, which is the
    /// documented synchronous-blocking behaviour rather than a hidden retry
    /// queue.
    fn write(self, bytes: &[u8]) -> Result<usize, StdioError> {
        let result = match self {
            Self::Stdout => {
                let stdout = std::io::stdout();
                write_all(stdout.lock(), bytes)
            }
            Self::Stderr => {
                let stderr = std::io::stderr();
                write_all(stderr.lock(), bytes)
            }
        };
        result.map_err(|source| StdioError::write(self, &source))?;
        Ok(bytes.len())
    }
}

/// Writes `bytes` completely and flushes the writer.
///
/// `Write::write_all` reports a would-block error instead of retrying, so the
/// loop is explicit. A zero-length write for a non-empty buffer is treated as a
/// hard failure rather than spinning forever.
fn write_all(mut writer: impl Write, bytes: &[u8]) -> std::io::Result<()> {
    let mut pending = bytes;
    while !pending.is_empty() {
        match writer.write(pending) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "the stream accepted no bytes",
                ));
            }
            Ok(written) => pending = &pending[written..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    writer.flush()
}

/// A validated `Uint8Array` and its byte length.
struct View<'s> {
    view: v8::Local<'s, v8::Uint8Array>,
    length: usize,
}

/// Validates a JavaScript value as an attached `Uint8Array`.
///
/// V8's internal type check is used directly, so `Symbol.hasInstance`,
/// `byteLength` getters, and proxy traps written in JavaScript are never
/// consulted. A detached view is rejected rather than treated as empty. Shared
/// and resizable backing buffers are accepted because no JavaScript runs between
/// reading the length and copying the bytes.
fn checked_view<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<View<'s>, StdioError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(StdioError::new(ErrorCode::Type));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(StdioError::new(ErrorCode::Detached));
    };
    if buffer.was_detached() {
        return Err(StdioError::new(ErrorCode::Detached));
    }
    let length = view.byte_length();
    Ok(View { view, length })
}

/// Copies the validated view's bytes into a new buffer.
///
/// The caller checks `length` against the relevant bound first, so an oversized
/// input is rejected without allocating or copying it.
fn copy_view(view: &View<'_>) -> Result<Vec<u8>, StdioError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(view.length)
        .map_err(|_| StdioError::new(ErrorCode::Limit))?;
    bytes.resize(view.length, 0);
    let written = view.view.copy_contents(&mut bytes);
    if written != view.length {
        // No JavaScript runs during this op, so the view cannot have changed.
        // A short copy would mean V8 considers it detached or out of bounds.
        return Err(StdioError::new(ErrorCode::Detached));
    }
    Ok(bytes)
}

/// Copies `[start, end)` out of a validated view.
///
/// The subview is built with V8's native constructor, not by calling
/// `%TypedArray%.prototype.subarray`, so `Symbol.species`, a replaced
/// `constructor`, and prototype methods cannot run JavaScript or observe the
/// backing buffer. `checked_view` has already rejected a detached buffer, and
/// no JavaScript runs before the copy.
fn copy_range<'s>(
    scope: &v8::PinScope<'s, '_>,
    view: &View<'s>,
    start: usize,
    end: usize,
) -> Result<Vec<u8>, StdioError> {
    if start > end || end > view.length {
        return Err(StdioError::new(ErrorCode::Argument));
    }
    let length = end - start;
    // The public paths never ask for more than MAX_BYTES, and this keeps a
    // direct op call from reaching `copy_contents`, which takes a 32-bit length,
    // with a larger range.
    if length > MAX_BYTES {
        return Err(StdioError::new(ErrorCode::Limit));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| StdioError::new(ErrorCode::Limit))?;
    bytes.resize(length, 0);
    if length == 0 {
        return Ok(bytes);
    }
    let Some(buffer) = view.view.buffer(scope) else {
        return Err(StdioError::new(ErrorCode::Detached));
    };
    let offset = view
        .view
        .byte_offset()
        .checked_add(start)
        .ok_or_else(|| StdioError::new(ErrorCode::Argument))?;
    let Some(subview) = v8::Uint8Array::new(scope, buffer, offset, length) else {
        return Err(StdioError::new(ErrorCode::Limit));
    };
    let written = subview.copy_contents(&mut bytes);
    if written != length {
        // No JavaScript runs during this op, so a short copy means V8 considers
        // the view detached or out of bounds after `byte_length`.
        return Err(StdioError::new(ErrorCode::Detached));
    }
    Ok(bytes)
}

/// Reads a non-negative integer without running JavaScript coercion.
fn integer<'s>(scope: &v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Option<usize> {
    if !value.is_number() {
        return None;
    }
    let number = value.number_value(scope)?;
    if !number.is_finite() || number.trunc() != number || number < 0.0 {
        return None;
    }
    Some(number as usize)
}

/// Reads the maximum accepted length, clamped to [`MAX_BYTES`].
///
/// The wrapper passes the remaining capacity, so a direct call to the op cannot
/// raise the documented bound.
fn checked_max_length<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<usize, StdioError> {
    let length = integer(scope, value).ok_or_else(|| StdioError::new(ErrorCode::Argument))?;
    if length > MAX_BYTES {
        return Err(StdioError::new(ErrorCode::Limit));
    }
    Ok(length)
}

/// Copies a `Uint8Array` or a string, rejecting anything larger than
/// `max_length` before allocating.
fn copy_input<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    max_length: usize,
) -> Result<Vec<u8>, StdioError> {
    if value.is_string() {
        return copy_string(scope, value, max_length);
    }
    let view = checked_view(scope, value)?;
    if view.length > max_length {
        return Err(StdioError::new(ErrorCode::Limit));
    }
    copy_view(&view)
}

/// Encodes a string as UTF-8 without a JavaScript encoder.
///
/// Untranslatable lone surrogates become U+FFFD, matching the usual JavaScript
/// UTF-8 encoder. The UTF-8 length is checked before the buffer is allocated.
fn copy_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    max_length: usize,
) -> Result<Vec<u8>, StdioError> {
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(StdioError::new(ErrorCode::Type));
    };
    let length = string.utf8_length(scope);
    if length > max_length {
        return Err(StdioError::new(ErrorCode::Limit));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| StdioError::new(ErrorCode::Limit))?;
    bytes.resize(length, 0);
    let written =
        string.write_utf8_v2(scope, &mut bytes, v8::WriteFlags::kReplaceInvalidUtf8, None);
    if written != length {
        return Err(StdioError::new(ErrorCode::Limit));
    }
    Ok(bytes)
}

/// Creates the null-prototype result object the JavaScript wrapper reads.
///
/// A null prototype means a script cannot intercept these property writes or
/// reads through `Object.prototype`.
fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: StdioError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "name", error.code.js_class());
    if let Some(code) = error.code.as_str() {
        set_string(scope, object, "code", code);
    }
    set_string(scope, object, "message", &error.message);
}

/// Wraps a Rust byte vector as a `Uint8Array` that owns its backing store.
fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        return v8::Uint8Array::new(scope, buffer, 0, 0);
    }
    let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

/// Returns copied bytes with the exact length the wrapper uses for accounting.
fn copy_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<u8>, StdioError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(bytes) => {
            let length = bytes.len();
            match bytes_to_v8(scope, bytes) {
                Some(value) => {
                    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                    set(scope, object, "value", value.into());
                    set(
                        scope,
                        object,
                        "length",
                        v8::Number::new(scope, length as f64).into(),
                    );
                }
                None => set_failure(scope, object, StdioError::new(ErrorCode::Limit)),
            }
        }
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

/// Returns a byte count.
fn number_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<usize, StdioError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(
                scope,
                object,
                "value",
                v8::Number::new(scope, value as f64).into(),
            );
        }
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

/// Copies an attached `Uint8Array`, or UTF-8 encodes a string, with a bound.
#[op2]
fn op_neo_stdio_copy<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    value: v8::Local<'s, v8::Value>,
    max_length: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let max_length = checked_max_length(scope, max_length)?;
        copy_input(scope, value, max_length)
    })();
    copy_outcome(scope, result)
}

/// Validates an attached `Uint8Array` and returns its byte length.
///
/// `Readable.readSync` uses this on its target before copying into it, so the
/// empty-target rule and the detached-view rejection are decided without
/// consulting JavaScript properties.
#[op2]
fn op_neo_stdio_view_length<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = checked_view(scope, value).map(|view| view.length);
    number_outcome(scope, result)
}

/// Validates a chunk and writes it to the current process's stdout or stderr.
/// Copies a byte range into a fresh `Uint8Array`.
///
/// `Readable.readSync` uses this on its private snapshot and `toUint8Array` uses
/// it on the writer's coalesced buffer. The bounds are validated here, and the
/// copy is native, so no `Symbol.species` lookup or prototype method runs.
#[op2]
fn op_neo_stdio_slice<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    value: v8::Local<'s, v8::Value>,
    start: v8::Local<'s, v8::Value>,
    end: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let view = checked_view(scope, value)?;
        let start = integer(scope, start).ok_or_else(|| StdioError::new(ErrorCode::Argument))?;
        let end = integer(scope, end).ok_or_else(|| StdioError::new(ErrorCode::Argument))?;
        if start > end || end > view.length {
            return Err(StdioError::new(ErrorCode::Argument));
        }
        copy_range(scope, &view, start, end)
    })();
    copy_outcome(scope, result)
}

#[op2]
fn op_neo_stdio_write<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    stream: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let stream = Stream::from_value(scope, stream)?;
        let bytes = copy_input(scope, value, MAX_BYTES)?;
        stream.write(&bytes)
    })();
    number_outcome(scope, result)
}

deno_core::extension!(
    neojs_stdio,
    ops = [
        op_neo_stdio_copy,
        op_neo_stdio_view_length,
        op_neo_stdio_slice,
        op_neo_stdio_write
    ],
    docs = "Synchronous byte-copy, range-copy, and process-standard-stream ops behind `Neo.getBuiltInModule(\"stdio\")`."
);

#[cfg(test)]
mod tests {
    use super::MAX_BYTES;
    use super::write_all;

    /// A writer that accepts nothing must fail instead of spinning forever.
    #[test]
    fn a_zero_length_write_is_an_error() {
        struct Refusing;
        impl std::io::Write for Refusing {
            fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let error = write_all(Refusing, b"bytes").expect_err("zero must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::WriteZero);
    }

    /// The documented bound is a plain literal in both languages.
    #[test]
    fn max_bytes_matches_the_javascript_constant() {
        assert_eq!(MAX_BYTES, 8_388_608);
    }
}
