//! Native `Vael` introspection object for the QuickJS runtime.
//!
//! The registry, the module list, and the native `vael:chars`, `vael:crypto`,
//! `vael:fs`, `vael:os`, `vael:ssh`, `vael:strings`, and `vael:uuid` (`vael:uuid`,
//! `vael:uuid/v4`, `vael:uuid/v7`) objects are Rust values. Shared ECMAScript
//! modules (`vael:assert`, `vael:console`, `vael:path`, `vael:slices`, and
//! `vael:test`) are constructed once per context from their shared factories in
//! `vaeljs-builtins` with native QuickJS host hooks.

#[cfg(any(feature = "vael-crypto", feature = "vael-test", feature = "vael-ssh"))]
use std::cell::RefCell;
#[cfg(feature = "vael-test")]
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
pub(super) struct VaelState {
    /// The frozen public `vael:assert` callable function, when selected.
    #[cfg(feature = "vael-assert")]
    pub(super) assert: Option<Persistent<Value<'static>>>,
    /// The frozen public `vael:chars` object, when the feature is selected.
    #[cfg(feature = "vael-chars")]
    pub(super) chars: Option<Persistent<Object<'static>>>,
    /// The live `globalThis.console` object captured at installation time, when
    /// `vael-console` is selected.
    #[cfg(feature = "vael-console")]
    pub(super) console: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:crypto` object, when the feature is selected.
    #[cfg(feature = "vael-crypto")]
    pub(super) crypto: Option<Persistent<Object<'static>>>,
    /// Frozen `Cipher.prototype` for `createEncryptor` / `createDecryptor`.
    #[cfg(feature = "vael-crypto")]
    pub(super) cipher_proto: Option<Persistent<Object<'static>>>,
    /// Frozen `Hash.prototype` for `createHash` / `createHmac`.
    #[cfg(feature = "vael-crypto")]
    pub(super) hash_proto: Option<Persistent<Object<'static>>>,
    /// Per-context bounded table of open stateful ciphers.
    #[cfg(feature = "vael-crypto")]
    pub(super) cipher_table: RefCell<vaeljs_crypto_core::cipher::CipherTable>,
    /// Per-context bounded table of open incremental hashes / HMACs.
    #[cfg(feature = "vael-crypto")]
    pub(super) digest_table: RefCell<vaeljs_crypto_core::hash::DigestTable>,
    /// Internal QuickJS class ID for `SharedArrayBuffer`, if available.
    #[cfg(feature = "vael-crypto")]
    pub(super) shared_array_buffer_class_id: Option<rquickjs::qjs::JSClassID>,
    /// The frozen public `vael:fs` object, when the feature is selected.
    #[cfg(feature = "vael-fs")]
    pub(super) fs: Option<Persistent<Object<'static>>>,
    /// The captured `%TypedArray%.prototype.byteLength` native getter.
    ///
    /// Used by `vael:crypto`, `vael:fs`, and `vael:ssh` to read the live byte
    /// length of length-tracking resizable-`ArrayBuffer` `Uint8Array` views
    /// without consulting mutable prototype properties.
    #[cfg(any(
        feature = "vael-crypto",
        feature = "vael-fs",
        feature = "vael-ssh",
        feature = "node-compat"
    ))]
    pub(super) typed_array_byte_length: Option<Persistent<Function<'static>>>,
    /// The frozen public `vael:os` object, when the feature is selected.
    #[cfg(feature = "vael-os")]
    pub(super) os: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:path` object, when the feature is selected.
    #[cfg(feature = "vael-path")]
    pub(super) path: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:slices` object, when the feature is selected.
    #[cfg(feature = "vael-slices")]
    pub(super) slices: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:ssh` object, when the feature is selected.
    #[cfg(feature = "vael-ssh")]
    pub(super) ssh: Option<Persistent<Object<'static>>>,
    /// Per-context handle table of open SSH connections, dropped when the
    /// QuickJS context drops.
    #[cfg(feature = "vael-ssh")]
    pub(super) ssh_state: RefCell<vaeljs_ssh_core::SshState>,
    /// The frozen public `vael:strings` object, when the feature is selected.
    #[cfg(feature = "vael-strings")]
    pub(super) strings: Option<Persistent<Object<'static>>>,
    /// The captured `String.prototype.slice` used for bounded text windows.
    ///
    /// The search binding calls this intrinsic instead of converting the whole
    /// string. A replacement made after runtime creation does not change it.
    #[cfg(feature = "vael-strings")]
    pub(super) slice: Option<Persistent<Function<'static>>>,
    /// The frozen public `vael:template` object, when the feature is selected.
    #[cfg(feature = "vael-template")]
    pub(super) template: Option<Persistent<Object<'static>>>,
    /// Frozen `Template.prototype` for `compile()`.
    #[cfg(feature = "vael-template")]
    pub(super) template_proto: Option<Persistent<Object<'static>>>,
    /// Frozen `TemplateSet.prototype` for `createTemplateSet()`.
    #[cfg(feature = "vael-template")]
    pub(super) template_set_proto: Option<Persistent<Object<'static>>>,
    /// Internal QuickJS class ID for plain/user objects (`JS_CLASS_OBJECT`).
    #[cfg(feature = "vael-template")]
    pub(super) object_class_id: rquickjs::qjs::JSClassID,
    /// Intrinsic `TypeError` constructor captured at installation time.
    #[cfg(feature = "vael-template")]
    pub(super) type_error_ctor: Option<Persistent<Function<'static>>>,
    /// Intrinsic `RangeError` constructor captured at installation time.
    #[cfg(feature = "vael-template")]
    pub(super) range_error_ctor: Option<Persistent<Function<'static>>>,
    /// The frozen public `vael:test` object, when the feature is selected.
    #[cfg(feature = "vael-test")]
    pub(super) test: Option<Persistent<Object<'static>>>,
    /// Installs the unhandled rejection handler on first `vael:test` access.
    #[cfg(feature = "vael-test")]
    pub(super) test_ensure_handler: Option<Persistent<Function<'static>>>,
    /// The JS unhandled rejection callback registered by `vael:test`.
    #[cfg(feature = "vael-test")]
    pub(super) rejection_handler: RefCell<Option<Persistent<Function<'static>>>>,
    /// The frozen public `vael:uuid` object, when the feature is selected.
    #[cfg(feature = "vael-uuid")]
    pub(super) uuid: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:uuid/v4` object, when the feature is selected.
    #[cfg(feature = "vael-uuid")]
    pub(super) uuid_v4: Option<Persistent<Object<'static>>>,
    /// The frozen public `vael:uuid/v7` object, when the feature is selected.
    #[cfg(feature = "vael-uuid")]
    pub(super) uuid_v7: Option<Persistent<Object<'static>>>,
    /// The frozen canonical module list returned for every call.
    pub(super) list: Persistent<Array<'static>>,
}

// `VaelState` holds no `'js` value directly: `Persistent` is runtime-rooted and
// carries no lifetime. The type is therefore safe to store as userdata.
unsafe impl<'js> JsLifetime<'js> for VaelState {
    type Changed<'to> = VaelState;
}

/// Installs the native `Vael` global and its registry.
///
/// The object and its properties are non-writable, non-enumerable, and
/// non-configurable. The registry returns the same object for every lookup and
/// `undefined` for every other string; a non-string name throws the intrinsic
/// `TypeError`.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    #[cfg(feature = "vael-test")] rejections: &Rc<RefCell<crate::runtime::RejectionState>>,
) -> Result<()> {
    #[cfg(feature = "vael-assert")]
    let assert: Option<Value<'js>> = Some(build_assert(ctx)?);
    #[cfg(feature = "vael-chars")]
    let chars: Option<Object<'js>> = Some(super::chars::object(ctx)?);
    #[cfg(feature = "vael-console")]
    let console: Option<Object<'js>> = Some(ctx.globals().get("console")?);
    #[cfg(feature = "vael-crypto")]
    let crypto: Option<Object<'js>> = Some(super::crypto::object(ctx)?);
    #[cfg(feature = "vael-crypto")]
    let cipher_proto: Option<Object<'js>> = Some(super::crypto::build_cipher_proto(ctx)?);
    #[cfg(feature = "vael-crypto")]
    let hash_proto: Option<Object<'js>> = Some(super::crypto::build_hash_proto(ctx)?);
    #[cfg(feature = "vael-crypto")]
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
    #[cfg(feature = "vael-fs")]
    let fs: Option<Object<'js>> = Some(super::fs::object(ctx)?);
    #[cfg(any(
        feature = "vael-crypto",
        feature = "vael-fs",
        feature = "vael-ssh",
        feature = "node-compat"
    ))]
    let typed_array_byte_length: Persistent<Function<'static>> = {
        let uint8_ctor: Object<'js> = ctx.globals().get("Uint8Array")?;
        let uint8_proto: Object<'js> = uint8_ctor.get("prototype")?;
        let ta_proto = super::native::get_prototype(ctx, uint8_proto.as_raw())?;
        let getter = super::native::own_getter(ctx, ta_proto.as_raw(), c"byteLength")?;
        Persistent::save(ctx, getter)
    };
    #[cfg(feature = "vael-os")]
    let os: Option<Object<'js>> = Some(super::os::object(ctx)?);
    #[cfg(feature = "vael-path")]
    let path: Option<Object<'js>> = Some(build_path(ctx)?);
    #[cfg(feature = "vael-slices")]
    let slices: Option<Object<'js>> = {
        let chars_obj = chars.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Some(build_slices(ctx, chars_obj)?)
    };
    #[cfg(feature = "vael-ssh")]
    let ssh: Option<Object<'js>> = Some(super::ssh::object(ctx)?);
    #[cfg(feature = "vael-strings")]
    let strings: Option<Object<'js>> = Some(super::strings::object(ctx)?);
    #[cfg(feature = "vael-strings")]
    let slice: Persistent<Function<'static>> = {
        let constructor: Object<'js> = ctx.globals().get("String")?;
        let prototype: Object<'js> = constructor.get("prototype")?;
        let function: Function<'js> = prototype.get("slice")?;
        Persistent::save(ctx, function)
    };
    #[cfg(feature = "vael-template")]
    let template: Option<Object<'js>> = Some(super::template::object(ctx)?);
    #[cfg(feature = "vael-template")]
    let template_proto: Option<Object<'js>> = Some(super::template::build_template_proto(ctx)?);
    #[cfg(feature = "vael-template")]
    let template_set_proto: Option<Object<'js>> =
        Some(super::template::build_template_set_proto(ctx)?);
    #[cfg(feature = "vael-template")]
    let object_class_id: rquickjs::qjs::JSClassID = {
        let obj = Object::new(ctx.clone())?;
        unsafe { rquickjs::qjs::JS_GetClassID(obj.as_raw()) }
    };
    #[cfg(feature = "vael-template")]
    let type_error_ctor: Persistent<Function<'static>> = {
        let ctor: Function<'js> = ctx.globals().get("TypeError")?;
        Persistent::save(ctx, ctor)
    };
    #[cfg(feature = "vael-template")]
    let range_error_ctor: Persistent<Function<'static>> = {
        let ctor: Function<'js> = ctx.globals().get("RangeError")?;
        Persistent::save(ctx, ctor)
    };
    #[cfg(feature = "vael-test")]
    let (test_module, test_ensure_handler) = build_test(ctx, rejections)?;
    #[cfg(feature = "vael-uuid")]
    let uuid_objects = super::uuid::objects(ctx)?;

    let list = Array::new(ctx.clone())?;
    #[cfg_attr(
        not(any(
            feature = "vael-assert",
            feature = "vael-chars",
            feature = "vael-console",
            feature = "vael-crypto",
            feature = "vael-fs",
            feature = "vael-os",
            feature = "vael-path",
            feature = "vael-slices",
            feature = "vael-ssh",
            feature = "vael-strings",
            feature = "vael-template",
            feature = "vael-test",
            feature = "vael-uuid"
        )),
        allow(unused_mut)
    )]
    let mut index = 0;
    #[cfg(feature = "vael-assert")]
    if assert.is_some() {
        list.set(index, vaeljs_builtins::assert::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-chars")]
    if chars.is_some() {
        list.set(index, vaeljs_builtins::chars::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-console")]
    if console.is_some() {
        list.set(index, vaeljs_builtins::console::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-crypto")]
    if crypto.is_some() {
        list.set(index, super::crypto::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-fs")]
    if fs.is_some() {
        list.set(index, vaeljs_fs_core::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-os")]
    if os.is_some() {
        list.set(index, vaeljs_builtins::os::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-path")]
    if path.is_some() {
        list.set(index, vaeljs_builtins::path::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-slices")]
    if slices.is_some() {
        list.set(index, vaeljs_builtins::slices::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-ssh")]
    if ssh.is_some() {
        list.set(index, vaeljs_ssh_core::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-strings")]
    if strings.is_some() {
        list.set(index, vaeljs_builtins::strings::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-template")]
    if template.is_some() {
        list.set(index, super::template::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-test")]
    {
        list.set(index, vaeljs_builtins::test::SPECIFIER)?;
        index += 1;
    }
    #[cfg(feature = "vael-uuid")]
    {
        list.set(index, vaeljs_builtins::uuid::SPECIFIER)?;
        index += 1;
        list.set(index, vaeljs_builtins::uuid::V4_SPECIFIER)?;
        index += 1;
        list.set(index, vaeljs_builtins::uuid::V7_SPECIFIER)?;
        index += 1;
    }
    let _ = index;
    freeze(ctx, list.as_object())?;

    ctx.store_userdata(VaelState {
        #[cfg(feature = "vael-assert")]
        assert: assert.map(|value| Persistent::save(ctx, value)),
        #[cfg(feature = "vael-chars")]
        chars: chars.map(|chars| Persistent::save(ctx, chars)),
        #[cfg(feature = "vael-console")]
        console: console.map(|console| Persistent::save(ctx, console)),
        #[cfg(feature = "vael-crypto")]
        crypto: crypto.map(|crypto| Persistent::save(ctx, crypto)),
        #[cfg(feature = "vael-crypto")]
        cipher_proto: cipher_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "vael-crypto")]
        hash_proto: hash_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "vael-crypto")]
        cipher_table: RefCell::new(vaeljs_crypto_core::cipher::CipherTable::default()),
        #[cfg(feature = "vael-crypto")]
        digest_table: RefCell::new(vaeljs_crypto_core::hash::DigestTable::default()),
        #[cfg(feature = "vael-crypto")]
        shared_array_buffer_class_id,
        #[cfg(feature = "vael-fs")]
        fs: fs.map(|fs| Persistent::save(ctx, fs)),
        #[cfg(any(
            feature = "vael-crypto",
            feature = "vael-fs",
            feature = "vael-ssh",
            feature = "node-compat"
        ))]
        typed_array_byte_length: Some(typed_array_byte_length),
        #[cfg(feature = "vael-os")]
        os: os.map(|os| Persistent::save(ctx, os)),
        #[cfg(feature = "vael-path")]
        path: path.map(|path| Persistent::save(ctx, path)),
        #[cfg(feature = "vael-slices")]
        slices: slices.map(|slices| Persistent::save(ctx, slices)),
        #[cfg(feature = "vael-ssh")]
        ssh: ssh.map(|ssh| Persistent::save(ctx, ssh)),
        #[cfg(feature = "vael-ssh")]
        ssh_state: RefCell::new(vaeljs_ssh_core::SshState::new()),
        #[cfg(feature = "vael-strings")]
        strings: strings.map(|strings| Persistent::save(ctx, strings)),
        #[cfg(feature = "vael-strings")]
        slice: Some(slice),
        #[cfg(feature = "vael-template")]
        template: template.map(|template| Persistent::save(ctx, template)),
        #[cfg(feature = "vael-template")]
        template_proto: template_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "vael-template")]
        template_set_proto: template_set_proto.map(|proto| Persistent::save(ctx, proto)),
        #[cfg(feature = "vael-template")]
        object_class_id,
        #[cfg(feature = "vael-template")]
        type_error_ctor: Some(type_error_ctor),
        #[cfg(feature = "vael-template")]
        range_error_ctor: Some(range_error_ctor),
        #[cfg(feature = "vael-test")]
        test: Some(Persistent::save(ctx, test_module)),
        #[cfg(feature = "vael-test")]
        test_ensure_handler: Some(Persistent::save(ctx, test_ensure_handler)),
        #[cfg(feature = "vael-test")]
        rejection_handler: RefCell::new(None),
        #[cfg(feature = "vael-uuid")]
        uuid: Some(Persistent::save(ctx, uuid_objects.uuid)),
        #[cfg(feature = "vael-uuid")]
        uuid_v4: Some(Persistent::save(ctx, uuid_objects.v4)),
        #[cfg(feature = "vael-uuid")]
        uuid_v7: Some(Persistent::save(ctx, uuid_objects.v7)),
        list: Persistent::save(ctx, list),
    })
    .map_err(|_| rquickjs::Error::Unknown)?;

    let vael = Object::new(ctx.clone())?;
    vael.prop("engine", "vael-lite")?;
    vael.prop("getBuiltInModule", Function::new(ctx.clone(), get_builtin)?)?;
    vael.prop(
        "getBuiltInModulesList",
        Function::new(ctx.clone(), get_builtin_list)?,
    )?;
    freeze(ctx, &vael)?;
    ctx.globals().prop("Vael", vael)?;
    Ok(())
}

#[cfg(feature = "vael-assert")]
fn build_assert<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let factory: Function<'js> = ctx.eval(vaeljs_builtins::assert::FACTORY_SOURCE)?;
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

#[cfg(feature = "vael-path")]
fn build_path<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(vaeljs_builtins::path::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set("isWindows", vaeljs_builtins::path::is_windows())?;
    host.set("readCwd", Function::new(ctx.clone(), path_read_cwd)?)?;
    factory.call((host,))
}

#[cfg(feature = "vael-path")]
fn path_read_cwd<'js>(ctx: Ctx<'js>) -> Result<Value<'js>> {
    match vaeljs_builtins::path::current_dir() {
        Some(text) => Ok(JsString::from_str(ctx, &text)?.into_value()),
        None => Ok(Value::new_null(ctx)),
    }
}

#[cfg(feature = "vael-slices")]
fn build_slices<'js>(ctx: &Ctx<'js>, chars: &Object<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(vaeljs_builtins::slices::FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    let to_upper: Value<'js> = chars.get("toUpper")?;
    let to_lower: Value<'js> = chars.get("toLower")?;
    host.set("toUpper", to_upper)?;
    host.set("toLower", to_lower)?;
    factory.call((host,))
}

#[cfg(feature = "vael-test")]
fn build_test<'js>(
    ctx: &Ctx<'js>,
    rejections: &Rc<RefCell<crate::runtime::RejectionState>>,
) -> Result<(Object<'js>, Function<'js>)> {
    let factory: Function<'js> = ctx.eval(vaeljs_builtins::test::FACTORY_SOURCE)?;
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

#[cfg(feature = "vael-test")]
fn test_set_rejection_handler<'js>(ctx: Ctx<'js>, handler: Opt<Value<'js>>) -> Result<()> {
    let state = ctx
        .userdata::<VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let Some(handler) = handler.0.and_then(|value| value.into_function()) else {
        return Err(Exception::throw_type(&ctx, "handler must be a function"));
    };
    *state.rejection_handler.borrow_mut() = Some(Persistent::save(&ctx, handler));
    Ok(())
}

#[cfg(feature = "vael-test")]
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
    let state = ctx
        .userdata::<VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
    #[cfg(feature = "vael-assert")]
    if name == vaeljs_builtins::assert::SPECIFIER {
        let Some(assert) = state.assert.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return assert.clone().restore(&ctx);
    }
    #[cfg(feature = "vael-chars")]
    if name == vaeljs_builtins::chars::SPECIFIER {
        let Some(chars) = state.chars.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(chars.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-console")]
    if name == vaeljs_builtins::console::SPECIFIER {
        let Some(console) = state.console.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(console.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-crypto")]
    if name == super::crypto::SPECIFIER {
        let Some(crypto) = state.crypto.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(crypto.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-fs")]
    if name == vaeljs_fs_core::SPECIFIER {
        let Some(fs) = state.fs.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(fs.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-os")]
    if name == vaeljs_builtins::os::SPECIFIER {
        let Some(os) = state.os.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(os.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-path")]
    if name == vaeljs_builtins::path::SPECIFIER {
        let Some(path) = state.path.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(path.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-slices")]
    if name == vaeljs_builtins::slices::SPECIFIER {
        let Some(slices) = state.slices.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(slices.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-ssh")]
    if name == vaeljs_ssh_core::SPECIFIER {
        let Some(ssh) = state.ssh.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(ssh.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-strings")]
    if name == vaeljs_builtins::strings::SPECIFIER {
        let Some(strings) = state.strings.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(strings.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-template")]
    if name == super::template::SPECIFIER {
        let Some(template) = state.template.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(template.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-test")]
    if name == vaeljs_builtins::test::SPECIFIER {
        if let Some(ensure) = state.test_ensure_handler.as_ref() {
            let ensure = ensure.clone().restore(&ctx)?;
            drop(state);
            let _: Value<'js> = ensure.call(())?;
            let state = ctx
                .userdata::<VaelState>()
                .ok_or(rquickjs::Error::Unknown)?;
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
    #[cfg(feature = "vael-uuid")]
    if name == vaeljs_builtins::uuid::SPECIFIER {
        let Some(uuid) = state.uuid.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-uuid")]
    if name == vaeljs_builtins::uuid::V4_SPECIFIER {
        let Some(uuid_v4) = state.uuid_v4.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid_v4.clone().restore(&ctx)?.into_value());
    }
    #[cfg(feature = "vael-uuid")]
    if name == vaeljs_builtins::uuid::V7_SPECIFIER {
        let Some(uuid_v7) = state.uuid_v7.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(uuid_v7.clone().restore(&ctx)?.into_value());
    }
    #[cfg(not(any(
        feature = "vael-assert",
        feature = "vael-chars",
        feature = "vael-console",
        feature = "vael-crypto",
        feature = "vael-fs",
        feature = "vael-os",
        feature = "vael-path",
        feature = "vael-slices",
        feature = "vael-ssh",
        feature = "vael-strings",
        feature = "vael-template",
        feature = "vael-test",
        feature = "vael-uuid"
    )))]
    let _ = (&state, &name);
    Ok(Value::new_undefined(ctx.clone()))
}

/// Returns the frozen canonical module list.
fn get_builtin_list<'js>(ctx: Ctx<'js>) -> Result<Value<'js>> {
    let state = ctx
        .userdata::<VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
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
