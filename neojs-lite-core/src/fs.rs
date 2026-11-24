//! Native `neo:fs` bindings for QuickJS.
//!
//! All core filesystem operations live in the engine-neutral [`neojs_fs_core`]
//! crate. This module builds one frozen `FsObject` per context with the 14
//! exports in [`neojs_fs_core::EXPORTS`] and shares those exact function objects
//! across `Neo.getBuiltInModule("neo:fs")` and `import ... from "neo:fs"`.

use std::mem::MaybeUninit;
use std::path::PathBuf;

use neojs_fs_core::{
    AtomicWriteOptions, CopyOptions, DirEntry, FileInfo, FsError, MkdirOptions, ReadOptions,
    RemoveOptions, WriteOptions,
};
use rquickjs::{
    Array, Ctx, Exception, Function, Object, Result, String as JsString, TypedArray, Value,
    function::Rest, module::ModuleDef, object::Property, qjs,
};

const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;

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

/// Creates the frozen public registry object for `neo:fs`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set(
        "readFile",
        Function::new(ctx.clone(), read_file)?
            .with_name("readFile")?
            .with_length(2)?,
    )?;
    object.set(
        "readTextFile",
        Function::new(ctx.clone(), read_text_file)?
            .with_name("readTextFile")?
            .with_length(2)?,
    )?;
    object.set(
        "writeFile",
        Function::new(ctx.clone(), write_file)?
            .with_name("writeFile")?
            .with_length(3)?,
    )?;
    object.set(
        "writeTextFile",
        Function::new(ctx.clone(), write_text_file)?
            .with_name("writeTextFile")?
            .with_length(3)?,
    )?;
    object.set(
        "writeFileAtomic",
        Function::new(ctx.clone(), write_file_atomic)?
            .with_name("writeFileAtomic")?
            .with_length(3)?,
    )?;
    object.set(
        "exists",
        Function::new(ctx.clone(), exists)?
            .with_name("exists")?
            .with_length(1)?,
    )?;
    object.set(
        "stat",
        Function::new(ctx.clone(), stat)?
            .with_name("stat")?
            .with_length(1)?,
    )?;
    object.set(
        "lstat",
        Function::new(ctx.clone(), lstat)?
            .with_name("lstat")?
            .with_length(1)?,
    )?;
    object.set(
        "readDir",
        Function::new(ctx.clone(), read_dir)?
            .with_name("readDir")?
            .with_length(1)?,
    )?;
    object.set(
        "mkdir",
        Function::new(ctx.clone(), mkdir)?
            .with_name("mkdir")?
            .with_length(2)?,
    )?;
    object.set(
        "remove",
        Function::new(ctx.clone(), remove)?
            .with_name("remove")?
            .with_length(2)?,
    )?;
    object.set(
        "rename",
        Function::new(ctx.clone(), rename)?
            .with_name("rename")?
            .with_length(2)?,
    )?;
    object.set(
        "copyFile",
        Function::new(ctx.clone(), copy_file)?
            .with_name("copyFile")?
            .with_length(3)?,
    )?;
    object.set(
        "realpath",
        Function::new(ctx.clone(), realpath)?
            .with_name("realpath")?
            .with_length(1)?,
    )?;
    freeze_native(ctx, &object)?;
    Ok(object)
}

fn freeze_native<'js>(ctx: &Ctx<'js>, object: &Object<'js>) -> Result<()> {
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

fn throw_fs_error<'js>(ctx: &Ctx<'js>, error: FsError) -> rquickjs::Error {
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
    if let Err(err) = object.prop(
        "message",
        Property::from(error.to_string()).writable().configurable(),
    ) {
        return err;
    }
    if let Err(err) = object.prop(
        "code",
        Property::from(error.code().as_str())
            .writable()
            .enumerable()
            .configurable(),
    ) {
        return err;
    }
    ctx.throw(object.into_value())
}

fn read_strict_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    surrogate_err: fn() -> FsError,
) -> Result<String> {
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, type_msg));
    }
    let js_str =
        JsString::from_value(value.clone()).map_err(|_| Exception::throw_type(ctx, type_msg))?;
    match js_str.to_string() {
        Ok(text) => Ok(text),
        Err(rquickjs::Error::FromJs { .. } | rquickjs::Error::Utf8(_)) => {
            Err(throw_fs_error(ctx, surrogate_err()))
        }
        Err(other) => Err(other),
    }
}

fn read_path<'js>(ctx: &Ctx<'js>, value: &Value<'js>, type_msg: &str) -> Result<String> {
    read_strict_string(ctx, value, type_msg, || FsError::InvalidPath {
        reason: "path contains an unpaired UTF-16 surrogate",
        source: None,
    })
}

fn read_text<'js>(ctx: &Ctx<'js>, value: &Value<'js>, type_msg: &str) -> Result<String> {
    read_strict_string(ctx, value, type_msg, || FsError::InvalidUtf8 {
        reason: "text contains an unpaired UTF-16 surrogate",
    })
}

fn read_uint8_array<'js>(ctx: &Ctx<'js>, value: &Value<'js>, type_msg: &str) -> Result<Vec<u8>> {
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
        return Err(Exception::throw_type(
            ctx,
            "Uint8Array buffer is detached or out of bounds",
        ));
    }
    let buf = unsafe { Value::from_raw(ctx.clone(), buf_raw) };

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
        .ok_or_else(|| {
            Exception::throw_type(ctx, "Uint8Array buffer is detached or out of bounds")
        })?;

    let offset = byte_offset as usize;
    let end = offset.checked_add(live_len).ok_or_else(|| {
        Exception::throw_type(ctx, "Uint8Array buffer is detached or out of bounds")
    })?;
    let mut buf_size: qjs::size_t = 0;
    let buf_ptr = unsafe { qjs::JS_GetArrayBuffer(raw_ctx, &mut buf_size, buf.as_raw()) };
    if buf_ptr.is_null() {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        return Err(Exception::throw_type(
            ctx,
            "Uint8Array buffer is detached or out of bounds",
        ));
    }
    if end > buf_size as usize {
        return Err(Exception::throw_type(
            ctx,
            "Uint8Array buffer is detached or out of bounds",
        ));
    }
    // SAFETY: `buf_ptr` is valid for `buf_size` bytes, `end <= buf_size`, and no
    // JavaScript executes while `slice` is alive.
    let slice = unsafe { &std::slice::from_raw_parts(buf_ptr, buf_size as usize)[offset..end] };
    let mut out = Vec::new();
    out.try_reserve_exact(slice.len()).map_err(|_| {
        throw_fs_error(
            ctx,
            FsError::Io {
                reason: "failed to allocate byte buffer".to_owned(),
                source: None,
            },
        )
    })?;
    out.extend_from_slice(slice);
    Ok(out)
}

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

fn parse_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
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
        return Err(Exception::throw_type(ctx, "options must be a plain object"));
    }

    let proto = get_prototype(ctx, value.as_raw())?;
    let plain = Object::new(ctx.clone())?;
    let plain_proto = get_prototype(ctx, plain.as_raw())?;
    if !proto.is_null() && proto != plain_proto {
        return Err(Exception::throw_type(ctx, "options must be a plain object"));
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
        return Err(Exception::throw_type(ctx, "options must be a plain object"));
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
        if found < 0 {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            return Err(Exception::throw_type(ctx, "options must be a plain object"));
        }
        if found == 0 {
            return Err(Exception::throw_type(ctx, "options must be a plain object"));
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
            .map_err(|_| Exception::throw_type(ctx, "options must be a plain object"))?;
        on_field(ctx, &key, &field_val)?;
    }
    Ok(())
}

fn opt_bool<'js>(ctx: &Ctx<'js>, val: &Value<'js>, target: &mut bool, msg: &str) -> Result<()> {
    if val.is_undefined() {
        return Ok(());
    }
    let Some(b) = val.as_bool() else {
        return Err(Exception::throw_type(ctx, msg));
    };
    *target = b;
    Ok(())
}

fn read_options<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<ReadOptions> {
    let mut opts = ReadOptions::default();
    parse_options(ctx, value, |ctx, key, val| match key {
        "maxBytes" => {
            if val.is_undefined() {
                return Ok(());
            }
            if !val.is_number() {
                return Err(Exception::throw_type(
                    ctx,
                    "options.maxBytes must be a number",
                ));
            }
            let Some(num) = val.as_number() else {
                return Err(Exception::throw_type(
                    ctx,
                    "options.maxBytes must be a number",
                ));
            };
            if !num.is_finite() || num.trunc() != num || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
                return Err(Exception::throw_range(
                    ctx,
                    "options.maxBytes must be a non-negative safe integer",
                ));
            }
            opts.max_bytes = Some(num as u64);
            Ok(())
        }
        _ => Err(Exception::throw_type(ctx, "unknown option key")),
    })?;
    Ok(opts)
}

fn write_options<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<WriteOptions> {
    let mut opts = WriteOptions::default();
    parse_options(ctx, value, |ctx, key, val| match key {
        "create" => opt_bool(
            ctx,
            val,
            &mut opts.create,
            "options.create must be a boolean",
        ),
        "createNew" => opt_bool(
            ctx,
            val,
            &mut opts.create_new,
            "options.createNew must be a boolean",
        ),
        "append" => opt_bool(
            ctx,
            val,
            &mut opts.append,
            "options.append must be a boolean",
        ),
        _ => Err(Exception::throw_type(ctx, "unknown option key")),
    })?;
    Ok(opts)
}

fn atomic_write_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
) -> Result<AtomicWriteOptions> {
    let mut opts = AtomicWriteOptions::default();
    parse_options(ctx, value, |ctx, key, val| match key {
        "sync" => opt_bool(ctx, val, &mut opts.sync, "options.sync must be a boolean"),
        "overwrite" => opt_bool(
            ctx,
            val,
            &mut opts.overwrite,
            "options.overwrite must be a boolean",
        ),
        _ => Err(Exception::throw_type(ctx, "unknown option key")),
    })?;
    Ok(opts)
}

fn recursive_option<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<bool> {
    let mut recursive = false;
    parse_options(ctx, value, |ctx, key, val| match key {
        "recursive" => opt_bool(
            ctx,
            val,
            &mut recursive,
            "options.recursive must be a boolean",
        ),
        _ => Err(Exception::throw_type(ctx, "unknown option key")),
    })?;
    Ok(recursive)
}

fn copy_options<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<CopyOptions> {
    let mut opts = CopyOptions::default();
    parse_options(ctx, value, |ctx, key, val| match key {
        "overwrite" => opt_bool(
            ctx,
            val,
            &mut opts.overwrite,
            "options.overwrite must be a boolean",
        ),
        _ => Err(Exception::throw_type(ctx, "unknown option key")),
    })?;
    Ok(opts)
}

fn file_info_to_js<'js>(ctx: &Ctx<'js>, info: FileInfo) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.prop(
        "isFile",
        Property::from(info.is_file)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    object.prop(
        "isDirectory",
        Property::from(info.is_directory)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    object.prop(
        "isSymlink",
        Property::from(info.is_symlink)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    object.prop(
        "size",
        Property::from(info.size as f64)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    let modified = match info.modified_ms {
        Some(ms) => Value::new_number(ctx.clone(), ms as f64),
        None => Value::new_null(ctx.clone()),
    };
    object.prop(
        "modifiedMs",
        Property::from(modified)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    object.prop(
        "readonly",
        Property::from(info.readonly)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    freeze_native(ctx, &object)?;
    Ok(object)
}

fn dir_entries_to_js<'js>(ctx: &Ctx<'js>, entries: Vec<DirEntry>) -> Result<Array<'js>> {
    let array = Array::new(ctx.clone())?;
    for (index, entry) in entries.into_iter().enumerate() {
        let item = Object::new(ctx.clone())?;
        item.prop(
            "name",
            Property::from(entry.name.as_str())
                .writable()
                .enumerable()
                .configurable(),
        )?;
        item.prop(
            "isFile",
            Property::from(entry.is_file)
                .writable()
                .enumerable()
                .configurable(),
        )?;
        item.prop(
            "isDirectory",
            Property::from(entry.is_directory)
                .writable()
                .enumerable()
                .configurable(),
        )?;
        item.prop(
            "isSymlink",
            Property::from(entry.is_symlink)
                .writable()
                .enumerable()
                .configurable(),
        )?;
        freeze_native(ctx, &item)?;
        array.as_object().prop(
            index as u32,
            Property::from(item).writable().enumerable().configurable(),
        )?;
    }
    Ok(array)
}

fn path_to_string(ctx: &Ctx<'_>, path: PathBuf) -> Result<String> {
    path.into_os_string().into_string().map_err(|_| {
        throw_fs_error(
            ctx,
            FsError::InvalidUtf8 {
                reason: "resolved path is not valid UTF-8",
            },
        )
    })
}

fn read_file<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<TypedArray<'js, u8>> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "readFile takes one or two arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let options = read_options(&ctx, args.0.get(1))?;
    let bytes = neojs_fs_core::read_file(path, options).map_err(|e| throw_fs_error(&ctx, e))?;
    TypedArray::<u8>::new(ctx, bytes)
}

fn read_text_file<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "readTextFile takes one or two arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let options = read_options(&ctx, args.0.get(1))?;
    neojs_fs_core::read_text_file(path, options).map_err(|e| throw_fs_error(&ctx, e))
}

fn write_file<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "writeFile takes two or three arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let bytes = read_uint8_array(&ctx, &args.0[1], "data must be a Uint8Array")?;
    let options = write_options(&ctx, args.0.get(2))?;
    neojs_fs_core::write_file(path, bytes, options).map_err(|e| throw_fs_error(&ctx, e))
}

fn write_text_file<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "writeTextFile takes two or three arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let text = read_text(&ctx, &args.0[1], "text must be a string")?;
    let options = write_options(&ctx, args.0.get(2))?;
    neojs_fs_core::write_text_file(path, text, options).map_err(|e| throw_fs_error(&ctx, e))
}

fn write_file_atomic<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "writeFileAtomic takes two or three arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let bytes = if args.0[1].is_string() {
        read_text(&ctx, &args.0[1], "data must be a Uint8Array or string")?.into_bytes()
    } else {
        read_uint8_array(&ctx, &args.0[1], "data must be a Uint8Array or string")?
    };
    let options = atomic_write_options(&ctx, args.0.get(2))?;
    neojs_fs_core::write_file_atomic(path, bytes, options).map_err(|e| throw_fs_error(&ctx, e))
}

fn exists<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<bool> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "exists takes exactly one argument",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    neojs_fs_core::exists(path).map_err(|e| throw_fs_error(&ctx, e))
}

fn stat<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "stat takes exactly one argument",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let info = neojs_fs_core::stat(path).map_err(|e| throw_fs_error(&ctx, e))?;
    file_info_to_js(&ctx, info)
}

fn lstat<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "lstat takes exactly one argument",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let info = neojs_fs_core::lstat(path).map_err(|e| throw_fs_error(&ctx, e))?;
    file_info_to_js(&ctx, info)
}

fn read_dir<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Array<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "readDir takes exactly one argument",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let entries = neojs_fs_core::read_dir(path).map_err(|e| throw_fs_error(&ctx, e))?;
    dir_entries_to_js(&ctx, entries)
}

fn mkdir<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "mkdir takes one or two arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let recursive = recursive_option(&ctx, args.0.get(1))?;
    neojs_fs_core::mkdir(path, MkdirOptions { recursive }).map_err(|e| throw_fs_error(&ctx, e))
}

fn remove<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "remove takes one or two arguments",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let recursive = recursive_option(&ctx, args.0.get(1))?;
    neojs_fs_core::remove(path, RemoveOptions { recursive }).map_err(|e| throw_fs_error(&ctx, e))
}

fn rename<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.len() != 2 {
        return Err(Exception::throw_type(
            &ctx,
            "rename takes exactly two arguments",
        ));
    }
    let from = read_path(&ctx, &args.0[0], "from must be a string")?;
    let to = read_path(&ctx, &args.0[1], "to must be a string")?;
    neojs_fs_core::rename(from, to).map_err(|e| throw_fs_error(&ctx, e))
}

fn copy_file<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "copyFile takes two or three arguments",
        ));
    }
    let from = read_path(&ctx, &args.0[0], "from must be a string")?;
    let to = read_path(&ctx, &args.0[1], "to must be a string")?;
    let options = copy_options(&ctx, args.0.get(2))?;
    neojs_fs_core::copy_file(from, to, options).map_err(|e| throw_fs_error(&ctx, e))
}

fn realpath<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "realpath takes exactly one argument",
        ));
    }
    let path = read_path(&ctx, &args.0[0], "path must be a string")?;
    let resolved = neojs_fs_core::realpath(path).map_err(|e| throw_fs_error(&ctx, e))?;
    path_to_string(&ctx, resolved)
}

/// The native QuickJS module served for `neo:fs`.
pub(super) struct FsModule;

impl ModuleDef for FsModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in neojs_fs_core::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(fs) = state.fs.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = fs.clone().restore(ctx)?;
        for name in neojs_fs_core::EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}
