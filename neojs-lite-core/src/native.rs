//! Shared QuickJS C-API helpers for native `neo:crypto`, `neo:fs`, and
//! `neo:ssh` bindings.
//!
//! Provides prototype-safe error construction (`JS_NewError` +
//! `[[DefineOwnProperty]]`), native `JS_FreezeObject`, strict plain-object
//! option parsing, and resizable-`ArrayBuffer`-aware `Uint8Array` reading.

use std::mem::MaybeUninit;

#[cfg(feature = "neo-crypto")]
use rquickjs::Array;
#[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
use rquickjs::TypedArray;
use rquickjs::{
    Ctx, Exception, Function, Object, Result, String as JsString, Value, object::Property, qjs,
};

#[cfg_attr(
    not(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh")),
    allow(dead_code)
)]
struct PropertyEnumGuard {
    raw: *mut qjs::JSContext,
    tab: *mut qjs::JSPropertyEnum,
    len: u32,
}

impl Drop for PropertyEnumGuard {
    fn drop(&mut self) {
        if !self.tab.is_null() {
            // SAFETY: `tab` and `len` were returned by `JS_GetOwnPropertyNames`
            // on the active `JSContext`.
            unsafe { qjs::JS_FreePropertyEnum(self.raw, self.tab, self.len) };
        }
    }
}

/// Freezes `object` using QuickJS's native `JS_FreezeObject` without
/// consulting `globalThis.Object.freeze`.
#[cfg_attr(
    not(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh")),
    allow(dead_code)
)]
pub(super) fn freeze_native<'js>(ctx: &Ctx<'js>, object: &Object<'js>) -> Result<()> {
    let rc = unsafe { qjs::JS_FreezeObject(ctx.as_raw().as_ptr(), object.as_raw()) };
    if rc < 0 {
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
    }
    Ok(())
}

/// Returns the internal `[[Prototype]]` of `val` via `JS_GetPrototype`.
pub(super) fn get_prototype<'js>(ctx: &Ctx<'js>, val: qjs::JSValue) -> Result<Value<'js>> {
    let proto_raw = unsafe { qjs::JS_GetPrototype(ctx.as_raw().as_ptr(), val) };
    if proto_raw.tag == i64::from(qjs::JS_TAG_EXCEPTION) {
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
    }
    Ok(unsafe { Value::from_raw(ctx.clone(), proto_raw) })
}

/// Reads an own data property from `object` without walking the prototype
/// chain or invoking getters/proxy traps.
///
/// Returns `Ok(None)` when the property is absent, or `Err(TypeError)` when
/// the own property is an accessor.
#[cfg(any(feature = "neo-crypto", feature = "node-compat"))]
#[cfg_attr(not(feature = "neo-crypto"), allow(dead_code))]
pub(super) fn own_data_property<'js>(
    ctx: &Ctx<'js>,
    object: qjs::JSValue,
    name: &std::ffi::CStr,
    accessor_msg: &str,
) -> Result<Option<Value<'js>>> {
    let raw = ctx.as_raw().as_ptr();
    let atom = unsafe { qjs::JS_NewAtom(raw, name.as_ptr()) };
    if atom == qjs::JS_ATOM_NULL {
        return Err(rquickjs::Error::Unknown);
    }
    own_data_property_by_atom(ctx, object, atom, accessor_msg)
}

/// Reads an own array element data property at `index` without walking
/// `Array.prototype` or invoking getters/proxy traps.
#[cfg(any(feature = "neo-crypto", feature = "node-compat"))]
pub(super) fn own_array_element<'js>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    index: u32,
    accessor_msg: &str,
) -> Result<Option<Value<'js>>> {
    let raw = ctx.as_raw().as_ptr();
    let atom = unsafe { qjs::JS_NewAtomUInt32(raw, index) };
    if atom == qjs::JS_ATOM_NULL {
        return Err(rquickjs::Error::Unknown);
    }
    own_data_property_by_atom(ctx, array.as_raw(), atom, accessor_msg)
}

#[cfg(any(feature = "neo-crypto", feature = "node-compat"))]
fn own_data_property_by_atom<'js>(
    ctx: &Ctx<'js>,
    object: qjs::JSValue,
    atom: qjs::JSAtom,
    accessor_msg: &str,
) -> Result<Option<Value<'js>>> {
    let raw = ctx.as_raw().as_ptr();
    let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
    let found = unsafe { qjs::JS_GetOwnProperty(raw, desc.as_mut_ptr(), object, atom) };
    unsafe { qjs::JS_FreeAtom(raw, atom) };
    if found < 0 {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(Exception::throw_type(ctx, accessor_msg));
    }
    if found == 0 {
        return Ok(None);
    }
    let desc = unsafe { desc.assume_init() };
    let value = unsafe { Value::from_raw(ctx.clone(), desc.value) };
    let _getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
    let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };
    if (desc.flags as u32 & qjs::JS_PROP_GETSET) != 0 {
        return Err(Exception::throw_type(ctx, accessor_msg));
    }
    Ok(Some(value))
}

/// Reads an own getter function from `object` without invoking user traps.
#[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
pub(super) fn own_getter<'js>(
    ctx: &Ctx<'js>,
    object: qjs::JSValue,
    name: &std::ffi::CStr,
) -> Result<Function<'js>> {
    let raw = ctx.as_raw().as_ptr();
    let atom = unsafe { qjs::JS_NewAtom(raw, name.as_ptr()) };
    if atom == qjs::JS_ATOM_NULL {
        return Err(rquickjs::Error::Unknown);
    }
    let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
    let found = unsafe { qjs::JS_GetOwnProperty(raw, desc.as_mut_ptr(), object, atom) };
    unsafe { qjs::JS_FreeAtom(raw, atom) };
    if found <= 0 {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(rquickjs::Error::Unknown);
    }
    let desc = unsafe { desc.assume_init() };
    let _value = unsafe { Value::from_raw(ctx.clone(), desc.value) };
    let getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
    let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };
    if (desc.flags as u32 & qjs::JS_PROP_GETSET) == 0 {
        return Err(rquickjs::Error::Unknown);
    }
    getter.into_function().ok_or(rquickjs::Error::Unknown)
}

/// Throws an intrinsic `Error` with own data properties `message`, `code`, and
/// optional `name` defined via `[[DefineOwnProperty]]` so hostile setters on
/// `Error.prototype` or `Object.prototype` cannot intercept them.
pub(super) fn throw_coded_error(
    ctx: &Ctx<'_>,
    name: Option<&str>,
    code: &str,
    message: &str,
) -> rquickjs::Error {
    let raw = unsafe { qjs::JS_NewError(ctx.as_raw().as_ptr()) };
    if raw.tag == i64::from(qjs::JS_TAG_EXCEPTION) {
        return if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        };
    }
    let value = unsafe { Value::from_raw(ctx.clone(), raw) };
    let Some(object) = value.into_object() else {
        return rquickjs::Error::Unknown;
    };
    if let Some(name) = name
        && let Err(err) = object.prop("name", Property::from(name).writable().configurable())
    {
        return err;
    }
    if let Err(err) = object.prop("message", Property::from(message).writable().configurable()) {
        return err;
    }
    if let Err(err) = object.prop(
        "code",
        Property::from(code).writable().enumerable().configurable(),
    ) {
        return err;
    }
    ctx.throw(object.into_value())
}

/// Validates a `Uint8Array` view (including resizable-`ArrayBuffer` views) and
/// passes its live byte slice to `f` without executing JavaScript while the
/// slice is alive.
#[cfg_attr(
    not(any(feature = "neo-fs", feature = "neo-ssh", feature = "node-compat")),
    allow(dead_code)
)]
pub(super) fn with_uint8_array_bytes<'js, T>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    f: impl FnOnce(&[u8]) -> Result<T>,
) -> Result<T> {
    with_uint8_array_bytes_ex(
        ctx,
        value,
        type_msg,
        |ctx| Exception::throw_type(ctx, "Uint8Array buffer is detached or out of bounds"),
        |slice, _is_shared| f(slice),
    )
}

/// Validates a `Uint8Array` view (including resizable-`ArrayBuffer` views),
/// reports detached/OOB views via `on_detached_or_oob`, and passes its live
/// byte slice and `is_shared` flag (`SharedArrayBuffer`-backed) to `f` without
/// executing JavaScript while the slice is alive.
#[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
pub(super) fn with_uint8_array_bytes_ex<'js, T>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    on_detached_or_oob: impl Fn(&Ctx<'js>) -> rquickjs::Error,
    f: impl FnOnce(&[u8], bool) -> Result<T>,
) -> Result<T> {
    let Ok(ta) = TypedArray::<u8>::from_value(value.clone()) else {
        return Err(Exception::throw_type(ctx, type_msg));
    };
    let raw_ctx = ctx.as_raw().as_ptr();
    let mut byte_offset: qjs::size_t = 0;
    let buf_raw = unsafe {
        qjs::JS_GetTypedArrayBuffer(
            raw_ctx,
            ta.as_value().as_raw(),
            &mut byte_offset,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if buf_raw.tag == i64::from(qjs::JS_TAG_EXCEPTION) {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(on_detached_or_oob(ctx));
    }
    let buf = unsafe { Value::from_raw(ctx.clone(), buf_raw) };

    #[cfg(feature = "neo-crypto")]
    let is_shared = {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .shared_array_buffer_class_id
            .is_some_and(|sab_id| unsafe { qjs::JS_GetClassID(buf.as_raw()) } == sab_id)
    };
    #[cfg(not(feature = "neo-crypto"))]
    let is_shared = false;

    let byte_length_getter = {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .typed_array_byte_length
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(ctx)?
    };
    let live_len_val: Value<'js> =
        byte_length_getter.call((rquickjs::function::This(ta.clone()),))?;
    let live_len = live_len_val
        .as_number()
        .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as usize)
        .ok_or_else(|| on_detached_or_oob(ctx))?;

    let offset = byte_offset as usize;
    let end = offset
        .checked_add(live_len)
        .ok_or_else(|| on_detached_or_oob(ctx))?;
    let mut buf_size: qjs::size_t = 0;
    let buf_ptr = unsafe { qjs::JS_GetArrayBuffer(raw_ctx, &mut buf_size, buf.as_raw()) };
    if buf_ptr.is_null() {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(on_detached_or_oob(ctx));
    }
    if end > buf_size as usize {
        return Err(on_detached_or_oob(ctx));
    }
    // SAFETY: `buf_ptr` is valid for `buf_size` bytes, `end <= buf_size`, and no
    // JavaScript executes while `slice` is alive.
    let slice = unsafe { &std::slice::from_raw_parts(buf_ptr, buf_size as usize)[offset..end] };
    f(slice, is_shared)
}

/// Validates that `value` (when `Some` and not `undefined`) is a plain object
/// with no proxies, symbols, non-enumerable properties, or accessors, and
/// passes each own enumerable string property to `on_field`.
#[cfg_attr(
    not(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh")),
    allow(dead_code)
)]
pub(super) fn parse_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    mut on_field: impl FnMut(&Ctx<'js>, &str, &Value<'js>) -> Result<()>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_undefined() {
        return Ok(());
    }
    let raw = ctx.as_raw().as_ptr();
    if !value.is_object()
        || value.is_proxy()
        || unsafe { qjs::JS_IsArray(value.as_raw()) }
        || unsafe { qjs::JS_IsFunction(raw, value.as_raw()) }
    {
        return Err(Exception::throw_type(ctx, type_msg));
    }

    let proto = get_prototype(ctx, value.as_raw())?;
    let plain = Object::new(ctx.clone())?;
    let plain_proto = get_prototype(ctx, plain.as_raw())?;
    if !proto.is_null() && proto != plain_proto {
        return Err(Exception::throw_type(ctx, type_msg));
    }

    let mut tab: *mut qjs::JSPropertyEnum = std::ptr::null_mut();
    let mut len: u32 = 0;
    let flags = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_SYMBOL_MASK) as std::ffi::c_int;
    let rc = unsafe { qjs::JS_GetOwnPropertyNames(raw, &mut tab, &mut len, value.as_raw(), flags) };
    let _guard = PropertyEnumGuard { raw, tab, len };
    if rc < 0 {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(Exception::throw_type(ctx, type_msg));
    }

    for i in 0..len {
        let atom = unsafe { (*tab.add(i as usize)).atom };
        let atom_val = unsafe { Value::from_raw(ctx.clone(), qjs::JS_AtomToValue(raw, atom)) };
        if atom_val.is_symbol() {
            return Err(Exception::throw_type(
                ctx,
                "options must not contain symbol properties",
            ));
        }

        let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
        let found = unsafe { qjs::JS_GetOwnProperty(raw, desc.as_mut_ptr(), value.as_raw(), atom) };
        if found <= 0 {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            return Err(Exception::throw_type(ctx, type_msg));
        }
        let desc = unsafe { desc.assume_init() };
        // Own the descriptor JSValues so they are freed on every exit path.
        let field_val = unsafe { Value::from_raw(ctx.clone(), desc.value) };
        let _getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
        let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };

        if (desc.flags as u32 & qjs::JS_PROP_ENUMERABLE) == 0 {
            return Err(Exception::throw_type(
                ctx,
                "options must not contain non-enumerable properties",
            ));
        }
        if (desc.flags as u32 & qjs::JS_PROP_GETSET) != 0 {
            return Err(Exception::throw_type(
                ctx,
                "options must not contain accessor properties",
            ));
        }
        let key = JsString::from_value(atom_val)
            .and_then(|s| s.to_string())
            .map_err(|_| Exception::throw_type(ctx, type_msg))?;
        on_field(ctx, &key, &field_val)?;
    }
    Ok(())
}
