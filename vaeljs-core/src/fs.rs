//! Core synchronous filesystem host ops for `vael:fs` on V8.
//!
//! All filesystem operations live in the engine-neutral [`vaeljs_fs_core`] crate.
//! This module validates JavaScript arguments (strings, attached `Uint8Array`
//! views, and own-property-only options objects), invokes `vaeljs_fs_core`, and
//! returns null-prototype result objects to [`crate::builtins`].

use std::path::PathBuf;

use deno_core::op2;
use deno_core::v8;
use vaeljs_fs_core::{
    AtomicWriteOptions, CopyOptions, DirEntry, FileInfo, FsError, MkdirOptions, ReadOptions,
    RemoveOptions, WriteOptions,
};

const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;

#[derive(Debug)]
enum BindingError {
    Type(&'static str),
    Range(&'static str),
    Fs(FsError),
}

impl From<FsError> for BindingError {
    fn from(err: FsError) -> Self {
        Self::Fs(err)
    }
}

/// Creates a null-prototype result object that cannot be intercepted via
/// `Object.prototype`.
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
    error: BindingError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    match error {
        BindingError::Type(message) => {
            set_string(scope, object, "name", "TypeError");
            set_string(scope, object, "message", message);
        }
        BindingError::Range(message) => {
            set_string(scope, object, "name", "RangeError");
            set_string(scope, object, "message", message);
        }
        BindingError::Fs(err) => {
            set_string(scope, object, "name", "Error");
            set_string(scope, object, "code", err.code().as_str());
            set_string(scope, object, "message", &err.to_string());
        }
    }
}

fn ok_unit<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    object.into()
}

fn outcome_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<v8::Local<'s, v8::Value>, BindingError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(scope, object, "value", value);
        }
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn outcome_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<(), BindingError>,
) -> v8::Local<'s, v8::Value> {
    match result {
        Ok(()) => ok_unit(scope),
        Err(error) => {
            let object = response(scope);
            set_failure(scope, object, error);
            object.into()
        }
    }
}

fn freeze_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<(), BindingError> {
    if object.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err(BindingError::Fs(FsError::Io {
            reason: "failed to freeze result object".to_owned(),
            source: None,
        }));
    }
    Ok(())
}

fn copy_strict_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
    surrogate_err: fn() -> FsError,
) -> Result<String, BindingError> {
    if !value.is_string() {
        return Err(BindingError::Type(type_msg));
    }
    let string =
        v8::Local::<v8::String>::try_from(value).map_err(|_| BindingError::Type(type_msg))?;
    let length = string.utf8_length(scope);
    let capacity = length.checked_add(4).ok_or_else(|| {
        BindingError::Fs(FsError::Io {
            reason: "string length overflow".to_owned(),
            source: None,
        })
    })?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|_| {
        BindingError::Fs(FsError::Io {
            reason: "failed to allocate string buffer".to_owned(),
            source: None,
        })
    })?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| BindingError::Fs(surrogate_err()))
}

fn read_path(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<String, BindingError> {
    copy_strict_string(scope, value, type_msg, || FsError::InvalidPath {
        reason: "path contains an unpaired UTF-16 surrogate",
        source: None,
    })
}

fn read_text(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<String, BindingError> {
    copy_strict_string(scope, value, type_msg, || FsError::InvalidUtf8 {
        reason: "text contains an unpaired UTF-16 surrogate",
    })
}

fn read_uint8_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<u8>, BindingError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(BindingError::Type(type_msg));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(BindingError::Type("Uint8Array buffer is detached"));
    };
    if buffer.was_detached() {
        return Err(BindingError::Type("Uint8Array buffer is detached"));
    }
    let length = view.byte_length();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| {
        BindingError::Fs(FsError::Io {
            reason: "failed to allocate byte buffer".to_owned(),
            source: None,
        })
    })?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err(BindingError::Type(
                "Uint8Array buffer is detached or out of bounds",
            ));
        }
    } else {
        // `v8::ArrayBufferView::copy_contents` panics when `dest.len() > i32::MAX`.
        // Copy directly from the off-heap backing store in safe Rust instead.
        let store = buffer.get_backing_store();
        let offset = view.byte_offset();
        let end = offset.checked_add(length).ok_or(BindingError::Type(
            "Uint8Array buffer is detached or out of bounds",
        ))?;
        let slice = store.get(offset..end).ok_or(BindingError::Type(
            "Uint8Array buffer is detached or out of bounds",
        ))?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    let length = bytes.len();
    let array = if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        v8::Uint8Array::new(scope, buffer, 0, 0)
    } else {
        let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
        let store = store.make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
        v8::Uint8Array::new(scope, buffer, 0, length)
    };
    array.map(Into::into).ok_or_else(|| {
        BindingError::Fs(FsError::Io {
            reason: "failed to allocate Uint8Array".to_owned(),
            source: None,
        })
    })
}

fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    let object = v8::Object::new(scope);
    object
        .get_prototype(scope)
        .ok_or(BindingError::Type("options must be a plain object"))
}

fn own_descriptor_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, BindingError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(BindingError::Type("options must be a plain object"));
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    match object.has_own_property(scope, key) {
        Some(true) => object
            .get(scope, key.into())
            .map(Some)
            .ok_or(BindingError::Type("options must be a plain object")),
        Some(false) => Ok(None),
        None => Err(BindingError::Type("options must be a plain object")),
    }
}

fn own_descriptor_boolean(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Result<bool, BindingError> {
    let Some(value) = own_descriptor_field(scope, object, name)? else {
        return Err(BindingError::Type("options must be a plain object"));
    };
    if !value.is_boolean() {
        return Err(BindingError::Type("options must be a plain object"));
    }
    Ok(value.boolean_value(scope))
}

/// Validates an optional `options` object without consulting inherited
/// properties, getters, or Proxy traps, and passes each validated own string
/// key/value pair to `on_field`.
fn parse_options<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    mut on_field: impl FnMut(
        &v8::PinScope<'s, '_>,
        &str,
        v8::Local<'s, v8::Value>,
    ) -> Result<(), BindingError>,
) -> Result<(), BindingError> {
    if value.is_undefined() {
        return Ok(());
    }
    if !value.is_object() || value.is_proxy() || value.is_array() || value.is_function() {
        return Err(BindingError::Type("options must be a plain object"));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| BindingError::Type("options must be a plain object"))?;
    let Some(prototype) = object.get_prototype(scope) else {
        return Err(BindingError::Type("options must be a plain object"));
    };
    if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
        return Err(BindingError::Type("options must be a plain object"));
    }

    let args = v8::GetPropertyNamesArgs {
        mode: v8::KeyCollectionMode::OwnOnly,
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        index_filter: v8::IndexFilter::IncludeIndices,
        key_conversion: v8::KeyConversionMode::ConvertToString,
    };
    let Some(names) = object.get_own_property_names(scope, args) else {
        return Err(BindingError::Type("options must be a plain object"));
    };

    for index in 0..names.length() {
        let Some(name_value) = names.get_index(scope, index) else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        if name_value.is_symbol() {
            return Err(BindingError::Type(
                "options must not contain symbol properties",
            ));
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        let Some(descriptor_value) = object.get_own_property_descriptor(scope, name) else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        if !own_descriptor_boolean(scope, descriptor, "enumerable")? {
            return Err(BindingError::Type(
                "options must not contain non-enumerable properties",
            ));
        }
        if own_descriptor_field(scope, descriptor, "get")?.is_some()
            || own_descriptor_field(scope, descriptor, "set")?.is_some()
        {
            return Err(BindingError::Type(
                "options must not contain accessor properties",
            ));
        }
        let Ok(key_str) = v8::Local::<v8::String>::try_from(name_value) else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        let key = key_str.to_rust_string_lossy(scope);
        let Some(field_value) = own_descriptor_field(scope, descriptor, "value")? else {
            return Err(BindingError::Type("options must be a plain object"));
        };
        on_field(scope, &key, field_value)?;
    }
    Ok(())
}

fn opt_bool(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    target: &mut bool,
    msg: &'static str,
) -> Result<(), BindingError> {
    if value.is_undefined() {
        return Ok(());
    }
    if !value.is_boolean() {
        return Err(BindingError::Type(msg));
    }
    *target = value.boolean_value(scope);
    Ok(())
}

fn read_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<ReadOptions, BindingError> {
    let mut opts = ReadOptions::default();
    parse_options(scope, value, |scope, key, val| match key {
        "maxBytes" => {
            if val.is_undefined() {
                return Ok(());
            }
            if !val.is_number() {
                return Err(BindingError::Type("options.maxBytes must be a number"));
            }
            let Some(num) = val.number_value(scope) else {
                return Err(BindingError::Type("options.maxBytes must be a number"));
            };
            if !num.is_finite() || num.trunc() != num || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
                return Err(BindingError::Range(
                    "options.maxBytes must be a non-negative safe integer",
                ));
            }
            opts.max_bytes = Some(num as u64);
            Ok(())
        }
        _ => Err(BindingError::Type("unknown option key")),
    })?;
    Ok(opts)
}

fn write_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<WriteOptions, BindingError> {
    let mut opts = WriteOptions::default();
    parse_options(scope, value, |scope, key, val| match key {
        "create" => opt_bool(
            scope,
            val,
            &mut opts.create,
            "options.create must be a boolean",
        ),
        "createNew" => opt_bool(
            scope,
            val,
            &mut opts.create_new,
            "options.createNew must be a boolean",
        ),
        "append" => opt_bool(
            scope,
            val,
            &mut opts.append,
            "options.append must be a boolean",
        ),
        _ => Err(BindingError::Type("unknown option key")),
    })?;
    Ok(opts)
}

fn atomic_write_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<AtomicWriteOptions, BindingError> {
    let mut opts = AtomicWriteOptions::default();
    parse_options(scope, value, |scope, key, val| match key {
        "sync" => opt_bool(scope, val, &mut opts.sync, "options.sync must be a boolean"),
        "overwrite" => opt_bool(
            scope,
            val,
            &mut opts.overwrite,
            "options.overwrite must be a boolean",
        ),
        _ => Err(BindingError::Type("unknown option key")),
    })?;
    Ok(opts)
}

fn recursive_option(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<bool, BindingError> {
    let mut recursive = false;
    parse_options(scope, value, |scope, key, val| match key {
        "recursive" => opt_bool(
            scope,
            val,
            &mut recursive,
            "options.recursive must be a boolean",
        ),
        _ => Err(BindingError::Type("unknown option key")),
    })?;
    Ok(recursive)
}

fn copy_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<CopyOptions, BindingError> {
    let mut opts = CopyOptions::default();
    parse_options(scope, value, |scope, key, val| match key {
        "overwrite" => opt_bool(
            scope,
            val,
            &mut opts.overwrite,
            "options.overwrite must be a boolean",
        ),
        _ => Err(BindingError::Type("unknown option key")),
    })?;
    Ok(opts)
}

fn file_info_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    info: FileInfo,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    let object = v8::Object::new(scope);
    set(
        scope,
        object,
        "isFile",
        v8::Boolean::new(scope, info.is_file).into(),
    );
    set(
        scope,
        object,
        "isDirectory",
        v8::Boolean::new(scope, info.is_directory).into(),
    );
    set(
        scope,
        object,
        "isSymlink",
        v8::Boolean::new(scope, info.is_symlink).into(),
    );
    set(
        scope,
        object,
        "size",
        v8::Number::new(scope, info.size as f64).into(),
    );
    let modified: v8::Local<'s, v8::Value> = match info.modified_ms {
        Some(ms) => v8::Number::new(scope, ms as f64).into(),
        None => v8::null(scope).into(),
    };
    set(scope, object, "modifiedMs", modified);
    set(
        scope,
        object,
        "readonly",
        v8::Boolean::new(scope, info.readonly).into(),
    );
    freeze_object(scope, object)?;
    Ok(object.into())
}

fn dir_entries_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: Vec<DirEntry>,
) -> Result<v8::Local<'s, v8::Value>, BindingError> {
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let item = v8::Object::new(scope);
        set_string(scope, item, "name", &entry.name);
        set(
            scope,
            item,
            "isFile",
            v8::Boolean::new(scope, entry.is_file).into(),
        );
        set(
            scope,
            item,
            "isDirectory",
            v8::Boolean::new(scope, entry.is_directory).into(),
        );
        set(
            scope,
            item,
            "isSymlink",
            v8::Boolean::new(scope, entry.is_symlink).into(),
        );
        freeze_object(scope, item)?;
        items.push(item.into());
    }
    Ok(v8::Array::new_with_elements(scope, &items).into())
}

fn path_to_string(path: PathBuf) -> Result<String, BindingError> {
    path.into_os_string().into_string().map_err(|_| {
        BindingError::Fs(FsError::InvalidUtf8 {
            reason: "resolved path is not valid UTF-8",
        })
    })
}

#[op2]
fn op_vael_fs_read_file<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let options = read_options(scope, options)?;
        let bytes = vaeljs_fs_core::read_file(path, options)?;
        bytes_to_v8(scope, bytes)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_read_text_file<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let options = read_options(scope, options)?;
        let text = vaeljs_fs_core::read_text_file(path, options)?;
        v8::String::new(scope, &text)
            .map(Into::into)
            .ok_or_else(|| {
                BindingError::Fs(FsError::Io {
                    reason: "failed to allocate JS string".to_owned(),
                    source: None,
                })
            })
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_write_file<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let bytes = read_uint8_array(scope, data, "data must be a Uint8Array")?;
        let options = write_options(scope, options)?;
        vaeljs_fs_core::write_file(path, bytes, options)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_write_text_file<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    text: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let text = read_text(scope, text, "text must be a string")?;
        let options = write_options(scope, options)?;
        vaeljs_fs_core::write_text_file(path, text, options)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_write_file_atomic<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let bytes = if data.is_string() {
            read_text(scope, data, "data must be a Uint8Array or string")?.into_bytes()
        } else {
            read_uint8_array(scope, data, "data must be a Uint8Array or string")?
        };
        let options = atomic_write_options(scope, options)?;
        vaeljs_fs_core::write_file_atomic(path, bytes, options)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_exists<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let found = vaeljs_fs_core::exists(path)?;
        Ok(v8::Boolean::new(scope, found).into())
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_stat<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let info = vaeljs_fs_core::stat(path)?;
        file_info_to_v8(scope, info)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_lstat<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let info = vaeljs_fs_core::lstat(path)?;
        file_info_to_v8(scope, info)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_read_dir<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let entries = vaeljs_fs_core::read_dir(path)?;
        dir_entries_to_v8(scope, entries)
    })();
    outcome_value(scope, result)
}

#[op2]
fn op_vael_fs_mkdir<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let recursive = recursive_option(scope, options)?;
        vaeljs_fs_core::mkdir(path, MkdirOptions { recursive })?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_remove<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let recursive = recursive_option(scope, options)?;
        vaeljs_fs_core::remove(path, RemoveOptions { recursive })?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_rename<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    from: v8::Local<'s, v8::Value>,
    to: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let from = read_path(scope, from, "from must be a string")?;
        let to = read_path(scope, to, "to must be a string")?;
        vaeljs_fs_core::rename(from, to)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_copy_file<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    from: v8::Local<'s, v8::Value>,
    to: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let from = read_path(scope, from, "from must be a string")?;
        let to = read_path(scope, to, "to must be a string")?;
        let options = copy_options(scope, options)?;
        vaeljs_fs_core::copy_file(from, to, options)?;
        Ok(())
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_fs_realpath<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    path: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let path = read_path(scope, path, "path must be a string")?;
        let resolved = path_to_string(vaeljs_fs_core::realpath(path)?)?;
        v8::String::new(scope, &resolved)
            .map(Into::into)
            .ok_or_else(|| {
                BindingError::Fs(FsError::Io {
                    reason: "failed to allocate JS string".to_owned(),
                    source: None,
                })
            })
    })();
    outcome_value(scope, result)
}

deno_core::extension!(
    vaeljs_fs,
    ops = [
        op_vael_fs_read_file,
        op_vael_fs_read_text_file,
        op_vael_fs_write_file,
        op_vael_fs_write_text_file,
        op_vael_fs_write_file_atomic,
        op_vael_fs_exists,
        op_vael_fs_stat,
        op_vael_fs_lstat,
        op_vael_fs_read_dir,
        op_vael_fs_mkdir,
        op_vael_fs_remove,
        op_vael_fs_rename,
        op_vael_fs_copy_file,
        op_vael_fs_realpath,
    ],
    docs = "Core synchronous filesystem operations behind `Vael.getBuiltInModule(\"vael:fs\")`."
);
