//! Native `Neo` introspection object for the QuickJS runtime.
//!
//! The registry, the module list, and the native `neo:chars`, `neo:crypto`,
//! `neo:fs`, `neo:os`, `neo:ssh`, `neo:strings`, and `neo:uuid` (`neo:uuid`,
//! `neo:uuid/v4`, `neo:uuid/v7`) objects are Rust values. Shared ECMAScript
//! modules (`neo:assert`, `neo:console`, `neo:path`, `neo:slices`, and
//! `neo:test`) are constructed once per context from their shared factories in
//! `neojs-builtins` with native QuickJS host hooks.

#[cfg(any(feature = "neo-crypto", feature = "neo-test", feature = "neo-ssh"))]
use std::cell::RefCell;
#[cfg(feature = "neo-test")]
use std::rc::Rc;

use rquickjs::{
    Array, Ctx, Exception, Function, JsLifetime, Object, Persistent, Result, String as JsString,
    Value, function::Opt,
};

/// The context-local native state shared by the registry and the native module.
///
/// It is stored as QuickJS userdata so user code cannot replace the function
/// objects between runtime creation and a later import. The persistent roots are
/// released when the context is dropped, while the runtime is still alive.
pub(super) struct NeoState {
    /// The frozen public `neo:assert` callable function, when selected.
    #[cfg(feature = "neo-assert")]
    pub(super) assert: Option<Persistent<Value<'static>>>,
    /// The frozen public `neo:chars` object, when the feature is selected.
    #[cfg(feature = "neo-chars")]
    pub(super) chars: Option<Persistent<Object<'static>>>,
    /// The live `globalThis.console` object captured at installation time, when
    /// `neo-console` is selected.
    #[cfg(feature = "neo-console")]
    pub(super) console: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:crypto` object, when the feature is selected.
    #[cfg(feature = "neo-crypto")]
    pub(super) crypto: Option<Persistent<Object<'static>>>,
    /// Frozen `Cipher.prototype` for `createEncryptor` / `createDecryptor`.
    #[cfg(feature = "neo-crypto")]
    pub(super) cipher_proto: Option<Persistent<Object<'static>>>,
    /// Frozen `Hash.prototype` for `createHash` / `createHmac`.
    #[cfg(feature = "neo-crypto")]
    pub(super) hash_proto: Option<Persistent<Object<'static>>>,
    /// Per-context bounded table of open stateful ciphers.
    #[cfg(feature = "neo-crypto")]
    pub(super) cipher_table: RefCell<neojs_crypto_core::cipher::CipherTable>,
    /// Per-context bounded table of open incremental hashes / HMACs.
    #[cfg(feature = "neo-crypto")]
    pub(super) digest_table: RefCell<neojs_crypto_core::hash::DigestTable>,
    /// Internal QuickJS class ID for `SharedArrayBuffer`, if available.
    #[cfg(feature = "neo-crypto")]
    pub(super) shared_array_buffer_class_id: Option<rquickjs::qjs::JSClassID>,
    /// The frozen public `neo:fs` object, when the feature is selected.
    #[cfg(feature = "neo-fs")]
    pub(super) fs: Option<Persistent<Object<'static>>>,
    /// The captured `%TypedArray%.prototype.byteLength` native getter.
    ///
    /// Used by `neo:crypto`, `neo:fs`, and `neo:ssh` to read the live byte
    /// length of length-tracking resizable-`ArrayBuffer` `Uint8Array` views
    /// without consulting mutable prototype properties.
    #[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
    pub(super) typed_array_byte_length: Option<Persistent<Function<'static>>>,
    /// The frozen public `neo:os` object, when the feature is selected.
    #[cfg(feature = "neo-os")]
    pub(super) os: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:path` object, when the feature is selected.
    #[cfg(feature = "neo-path")]
    pub(super) path: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:slices` object, when the feature is selected.
    #[cfg(feature = "neo-slices")]
    pub(super) slices: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:ssh` object, when the feature is selected.
    #[cfg(feature = "neo-ssh")]
    pub(super) ssh: Option<Persistent<Object<'static>>>,
    /// Per-context handle table of open SSH connections, dropped when the
    /// QuickJS context drops.
    #[cfg(feature = "neo-ssh")]
    pub(super) ssh_state: RefCell<neojs_ssh_core::SshState>,
    /// The frozen public `neo:strings` object, when the feature is selected.
    #[cfg(feature = "neo-strings")]
    pub(super) strings: Option<Persistent<Object<'static>>>,
    /// The captured `String.prototype.slice` used for bounded text windows.
    ///
    /// The search binding calls this intrinsic instead of converting the whole
    /// string. A replacement made after runtime creation does not change it.
    #[cfg(feature = "neo-strings")]
    pub(super) slice: Option<Persistent<Function<'static>>>,
    /// The frozen public `neo:test` object, when the feature is selected.
    #[cfg(feature = "neo-test")]
    pub(super) test: Option<Persistent<Object<'static>>>,
    /// Installs the unhandled rejection handler on first `neo:test` access.
    #[cfg(feature = "neo-test")]
    pub(super) test_ensure_handler: Option<Persistent<Function<'static>>>,
    /// The JS unhandled rejection callback registered by `neo:test`.
    #[cfg(feature = "neo-test")]
    pub(super) rejection_handler: RefCell<Option<Persistent<Function<'static>>>>,
    /// The frozen public `neo:uuid` object, when the feature is selected.
    #[cfg(feature = "neo-uuid")]
    pub(super) uuid: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:uuid/v4` object, when the feature is selected.
    #[cfg(feature = "neo-uuid")]
    pub(super) uuid_v4: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:uuid/v7` object, when the feature is selected.
    #[cfg(feature = "neo-uuid")]
    pub(super) uuid_v7: Option<Persistent<Object<'static>>>,
    /// The frozen canonical module list returned for every call.
    pub(super) list: Persistent<Array<'static>>,
}

// `NeoState` holds no `'js` value directly: `Persistent` is runtime-rooted and
// carries no lifetime. The type is therefore safe to store as userdata.
unsafe impl<'js> JsLifetime<'js> for NeoState {
    type Changed<'to> = NeoState;
}

/// Installs the native `Neo` global and its registry.
///
/// The object and its properties are non-writable, non-enumerable, and
/// non-configurable. The registry returns the same object for every lookup and
/// `undefined` for every other string; a non-string name throws the intrinsic
/// `TypeError`.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    #[cfg(feature = "neo-test")] rejections: &Rc<RefCell<crate::runtime::RejectionState>>,
) -> Result<()> {
    #[cfg(feature = "neo-assert")]
    let assert: Option<Value<'js>> = Some(build_assert(ctx)?);
    #[cfg(feature = "neo-chars")]
    let chars: Option<Object<'js>> = Some(super::chars::object(ctx)?);
    #[cfg(feature = "neo-console")]
    let console: Option<Object<'js>> = Some(ctx.globals().get("console")?);
    #[cfg(feature = "neo-crypto")]
    let crypto: Option<Object<'js>> = Some(super::crypto::object(ctx)?);
    #[cfg(feature = "neo-crypto")]
    let cipher_proto: Option<Object<'js>> = Some(super::crypto::build_cipher_proto(ctx)?);
    #[cfg(feature = "neo-crypto")]
    let hash_proto: Option<Object<'js>> = Some(super::crypto::build_hash_proto(ctx)?);
    #[cfg(feature = "neo-crypto")]
    let shared_array_buffer_class_id: Option<rquickjs::qjs::JSClassID> = {
        let sab_val: Value<'js> = ctx.eval(
            "typeof SharedArrayBuffer === 'function' ? new SharedArrayBuffer(0) : undefined",
        )?;
        if sab_val.is_object() {
            Some(unsafe { rquickjs::qjs::JS_GetClassID(sab_val.as_raw()) })
        } else {
            None
        }
    };
    #[cfg(feature = "neo-fs")]
    let fs: Option<Object<'js>> = Some(super::fs::object(ctx)?);
    #[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
    let typed_array_byte_length: Persistent<Function<'static>> = {
        let uint8_ctor: Object<'js> = ctx.globals().get("Uint8Array")?;
        let uint8_proto: Object<'js> = uint8_ctor.get("prototype")?;
        let ta_proto = super::native::get_prototype(ctx, uint8_proto.as_raw())?;
        let getter = super::native::own_getter(ctx, ta_proto.as_raw(), c"byteLength")?;
        Persistent::save(ctx, getter)
    };
    #[cfg(feature = "neo-os")]
    let os: Option<Object<'js>> = Some(super::os::object(ctx)?);
    #[cfg(feature = "neo-path")]
    let path: Option<Object<'js>> = Some(build_path(ctx)?);
    #[cfg(feature = "neo-slices")]
    let slices: Option<Object<'js>> = {
        let chars_obj = chars.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Some(build_slices(ctx, chars_obj)?)
    };
    #[cfg(feature = "neo-ssh")]
    let ssh: Option<Object<'js>> = Some(super::ssh::object(ctx)?);
    #[cfg(feature = "neo-strings")]
    let strings: Option<Object<'js>> = Some(super::strings::object(ctx)?);
    #[cfg(feature = "neo-strings")]
    let slice: Persistent<Function<'static>> = {
        let constructor: Object<'js> = ctx.globals().get("String")?;
        let prototype: Object<'js> = constructor.get("prototype")?;
        let function: Function<'js> = prototype.get("slice")?;
        Persistent::save(ctx, function)
    };
    #[cfg(feature = "neo-test")]
    let (test_module, test_ensure_handler) = build_test(ctx, rejections)?;
    #[cfg(feature = "neo-uuid")]
    let uuid_objects = super::uuid::objects(ctx)?;

    let list = Array::new(ctx.clone())?;
    #[cfg_attr(
        not(any(
            feature = "neo-assert",
            feature = "neo-chars",
            feature = "neo-console",
            feature = "neo-crypto",
            feature = "neo-fs",
            feature = "neo-os",
            feature = "neo-path",
            feature = "neo-slices",
            feature = "neo-ssh",
            feature = "neo-strings",
            feature = "neo-test",
            feature = "neo-uuid"
        )),
        allow(unused_mut)
    )]
    let mut index = 0;
    #[cfg(feature = "neo-assert")]
    if assert.is_some() {
        list.set(index, neojs_builtins::assert::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-chars")]
    if chars.is_some() {
        list.set(index, neojs_builtins::chars::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-console")]
    if console.is_some() {
        list.set(index, neojs_builtins::console::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-crypto")]
    if crypto.is_some() {
        list.set(index, super::crypto::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-fs")]
    if fs.is_some() {
        list.set(index, neojs_fs_core::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-os")]
    if os.is_some() {
        list.set(index, neojs_builtins::os::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-path")]
    if path.is_some() {
        list.set(index, neojs_builtins::path::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-slices")]
    if slices.is_some() {
        list.set(index, neojs_builtins::slices::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-ssh")]
    if ssh.is_some() {
        list.set(index, neojs_ssh_core::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-strings")]
    if strings.is_some() {
        list.set(index, neojs_builtins::strings::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-test")]
    {
        list.set(index, neojs_builtins::test::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "neo-uuid")]
    {
        list.set(index, neojs_builtins::uuid::SPECIFIER)?;
        index += 1;
        list.set(index, neojs_builtins::uuid::V4_SPECIFIER)?;
        index += 1;
        list.set(index, neojs_builtins::uuid::V7_SPECIFIER)?;
        index += 1;
    }
    let _ = index;
    freeze(ctx, list.as_object())?;

    ctx.store_userdata(NeoState {
        #[cfg(feature = "neo-assert")]
        assert: assert.map(|value| Persistent::save(ctx, value)),
        #[cfg(feature = "neo-chars")]
        chars: chars.map(|chars| Persistent::save(ctx, chars)),
        #[cfg(feature = "neo-console")]
        console: console.map(|console| Persistent::save(ctx, console)),
        #[cfg(feature = "neo-crypto")]
        crypto: crypto.map(|crypto| Persistent::save(ctx, crypto)),
        #[cfg(feature = "neo-crypto")]
        cipher_proto: cipher_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "neo-crypto")]
        hash_proto: hash_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "neo-crypto")]
        cipher_table: RefCell::new(neojs_crypto_core::cipher::CipherTable::default()),
        #[cfg(feature = "neo-crypto")]
        digest_table: RefCell::new(neojs_crypto_core::hash::DigestTable::default()),
        #[cfg(feature = "neo-crypto")]
        shared_array_buffer_class_id,
        #[cfg(feature = "neo-fs")]
        fs: fs.map(|fs| Persistent::save(ctx, fs)),
        #[cfg(any(feature = "neo-crypto", feature = "neo-fs", feature = "neo-ssh"))]
        typed_array_byte_length: Some(typed_array_byte_length),
        #[cfg(feature = "neo-os")]
        os: os.map(|os| Persistent::save(ctx, os)),
        #[cfg(feature = "neo-path")]
        path: path.map(|path| Persistent::save(ctx, path)),
        #[cfg(feature = "neo-slices")]
        slices: slices.map(|slices| Persistent::save(ctx, slices)),
        #[cfg(feature = "neo-ssh")]
        ssh: ssh.map(|ssh| Persistent::save(ctx, ssh)),
        #[cfg(feature = "neo-ssh")]
        ssh_state: RefCell::new(neojs_ssh_core::SshState::new()),
        #[cfg(feature = "neo-strings")]
        strings: strings.map(|strings| Persistent::save(ctx, strings)),
        #[cfg(feature = "neo-strings")]
        slice: Some(slice),
        #[cfg(feature = "neo-test")]
        test: Some(Persistent::save(ctx, test_module)),
        #[cfg(feature = "neo-test")]
        test_ensure_handler: Some(Persistent::save(ctx, test_ensure_handler)),
        #[cfg(feature = "neo-test")]
        rejection_handler: RefCell::new(None),
        #[cfg(feature = "neo-uuid")]
        uuid: Some(Persistent::save(ctx, uuid_objects.uuid)),
        #[cfg(feature = "neo-uuid")]
        uuid_v4: Some(Persistent::save(ctx, uuid_objects.v4)),
        #[cfg(feature = "neo-uuid")]
        uuid_v7: Some(Persistent::save(ctx, uuid_objects.v7)),
        list: Persistent::save(ctx, list),
    })
    .map_err(|_| rquickjs::Error::Unknown)?;

    let neo = Object::new(ctx.clone())?;
    neo.prop("engine", "neo-lite")?;
    neo.prop("getBuiltInModule", Function::new(ctx.clone(), get_builtin)?)?;
    neo.prop(
        "getBuiltInModulesList",
        Function::new(ctx.clone(), get_builtin_list)?,
    )?;
    freeze(ctx, &neo)?;
    ctx.globals().prop("Neo", neo)?;
    Ok(())
}

#[cfg(feature = "neo-assert")]
fn build_assert<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let factory: Function<'js> = ctx.eval(neojs_builtins::assert::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set(
        "isNativePromise",
        Function::new(ctx.clone(), crate::promise::is_native_promise)?,
    )?;
    host.set(
        "observePromise",
        Function::new(ctx.clone(), crate::promise::observe_promise)?,
    )?;
    factory.call((host,))
}

#[cfg(feature = "neo-path")]
fn build_path<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(neojs_builtins::path::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set("isWindows", neojs_builtins::path::is_windows())?;
    host.set("readCwd", Function::new(ctx.clone(), path_read_cwd)?)?;
    factory.call((host,))
}

#[cfg(feature = "neo-path")]
fn path_read_cwd<'js>(ctx: Ctx<'js>) -> Result<Value<'js>> {
    match neojs_builtins::path::current_dir() {
        Some(text) => Ok(JsString::from_str(ctx, &text)?.into_value()),
        None => Ok(Value::new_null(ctx)),
    }
}

#[cfg(feature = "neo-slices")]
fn build_slices<'js>(ctx: &Ctx<'js>, chars: &Object<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(neojs_builtins::slices::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    let to_upper: Value<'js> = chars.get("toUpper")?;
    let to_lower: Value<'js> = chars.get("toLower")?;
    host.set("toUpper", to_upper)?;
    host.set("toLower", to_lower)?;
    factory.call((host,))
}

#[cfg(feature = "neo-test")]
fn build_test<'js>(
    ctx: &Ctx<'js>,
    rejections: &Rc<RefCell<crate::runtime::RejectionState>>,
) -> Result<(Object<'js>, Function<'js>)> {
    let factory: Function<'js> = ctx.eval(neojs_builtins::test::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set(
        "isNativePromise",
        Function::new(ctx.clone(), crate::promise::is_native_promise)?,
    )?;
    host.set(
        "observePromise",
        Function::new(ctx.clone(), crate::promise::observe_promise)?,
    )?;
    host.set(
        "print",
        Function::new(ctx.clone(), crate::runtime::host_print)?,
    )?;
    host.set(
        "setUnhandledRejectionHandler",
        Function::new(ctx.clone(), test_set_rejection_handler)?,
    )?;
    host.set(
        "eventLoopHasMoreWork",
        Function::new(ctx.clone(), test_is_job_pending)?,
    )?;
    let flush_rejections = Rc::clone(rejections);
    host.set(
        "flushUnhandledRejections",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> Result<()> {
            crate::runtime::flush_unhandled_rejections(&ctx, &flush_rejections)
        })?,
    )?;
    let harness: Object<'js> = factory.call((host,))?;
    let module: Object<'js> = harness.get("module")?;
    let ensure_handler: Function<'js> = harness.get("ensureHandler")?;
    Ok((module, ensure_handler))
}

#[cfg(feature = "neo-test")]
fn test_set_rejection_handler<'js>(ctx: Ctx<'js>, handler: Opt<Value<'js>>) -> Result<()> {
    let state = ctx.userdata::<NeoState>().ok_or(rquickjs::Error::Unknown)?;
    let Some(handler) = handler.0.and_then(|value| value.into_function()) else {
        return Err(Exception::throw_type(&ctx, "handler must be a function"));
    };
    *state.rejection_handler.borrow_mut() = Some(Persistent::save(&ctx, handler));
    Ok(())
}

#[cfg(feature = "neo-test")]
fn test_is_job_pending(ctx: Ctx<'_>) -> bool {
    crate::runtime::is_job_pending(&ctx)
}

/// Returns a built-in module by canonical specifier.
fn get_builtin<'js>(ctx: Ctx<'js>, name: Opt<Value<'js>>) -> Result<Value<'js>> {
    let Some(value) = name.0 else {
        return Err(Exception::throw_type(&ctx, "module name must be a string"));
    };
    if !value.is_string() {
        return Err(Exception::throw_type(&ctx, "module name must be a string"));
    }
    let name = JsString::from_value(value)?.to_string()?;
    let state = ctx.userdata::<NeoState>().ok_or(rquickjs::Error::Unknown)?;
    #[cfg(feature = "neo-assert")]
    if name == neojs_builtins::assert::SPECIFIER {
        let Some(assert) = state.assert.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return assert.clone().restore(&ctx);
    }
    #[cfg(feature = "neo-chars")]
    if name == neojs_builtins::chars::SPECIFIER {
        let Some(chars) = state.chars.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(chars.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-console")]
    if name == neojs_builtins::console::SPECIFIER {
        let Some(console) = state.console.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(console.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-crypto")]
    if name == super::crypto::SPECIFIER {
        let Some(crypto) = state.crypto.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(crypto.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-fs")]
    if name == neojs_fs_core::SPECIFIER {
        let Some(fs) = state.fs.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(fs.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-os")]
    if name == neojs_builtins::os::SPECIFIER {
        let Some(os) = state.os.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(os.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-path")]
    if name == neojs_builtins::path::SPECIFIER {
        let Some(path) = state.path.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(path.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-slices")]
    if name == neojs_builtins::slices::SPECIFIER {
        let Some(slices) = state.slices.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(slices.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-ssh")]
    if name == neojs_ssh_core::SPECIFIER {
        let Some(ssh) = state.ssh.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(ssh.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-strings")]
    if name == neojs_builtins::strings::SPECIFIER {
        let Some(strings) = state.strings.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(strings.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-test")]
    if name == neojs_builtins::test::SPECIFIER {
        if let Some(ensure) = state.test_ensure_handler.as_ref() {
            let ensure = ensure.clone().restore(&ctx)?;
            drop(state);
            let _: Value<'js> = ensure.call(())?;
            let state = ctx.userdata::<NeoState>().ok_or(rquickjs::Error::Unknown)?;
            let Some(test) = state.test.as_ref() else {
                return Ok(Value::new_undefined(ctx.clone()));
            };
            return Ok(test.clone().restore(&ctx)?.into_value());
        }
        let Some(test) = state.test.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(test.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-uuid")]
    if name == neojs_builtins::uuid::SPECIFIER {
        let Some(uuid) = state.uuid.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-uuid")]
    if name == neojs_builtins::uuid::V4_SPECIFIER {
        let Some(uuid_v4) = state.uuid_v4.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid_v4.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "neo-uuid")]
    if name == neojs_builtins::uuid::V7_SPECIFIER {
        let Some(uuid_v7) = state.uuid_v7.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid_v7.clone().restore(&ctx)?.into_value());
    }
    #[cfg(not(any(
        feature = "neo-assert",
        feature = "neo-chars",
        feature = "neo-console",
        feature = "neo-crypto",
        feature = "neo-fs",
        feature = "neo-os",
        feature = "neo-path",
        feature = "neo-slices",
        feature = "neo-ssh",
        feature = "neo-strings",
        feature = "neo-test",
        feature = "neo-uuid"
    )))]
    let _ = (&state, &name);
    Ok(Value::new_undefined(ctx.clone()))
}

/// Returns the frozen canonical module list.
fn get_builtin_list<'js>(ctx: Ctx<'js>) -> Result<Value<'js>> {
    let state = ctx.userdata::<NeoState>().ok_or(rquickjs::Error::Unknown)?;
    Ok(state.list.clone().restore(&ctx)?.into_value())
}

/// Freezes an object with the realm's `Object.freeze` intrinsic.
///
/// This is a native call to a language intrinsic, not an ECMAScript module
/// implementation. The intrinsic is looked up at installation time; replacing
/// `Object.freeze` later cannot change the result.
pub(super) fn freeze<'js>(ctx: &Ctx<'js>, object: &Object<'js>) -> Result<()> {
    let object_constructor: Object<'js> = ctx.globals().get("Object")?;
    let freeze: Function<'js> = object_constructor.get("freeze")?;
    let _: Value<'js> = freeze.call((object.clone(),))?;
    Ok(())
}
