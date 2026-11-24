//! Native `Neo` introspection object for the QuickJS runtime.
//!
//! The registry, the module list, and the native `neo:chars`, `neo:fs`,
//! `neo:os`, `neo:strings`, and `neo:uuid` (`neo:uuid`, `neo:uuid/v4`,
//! `neo:uuid/v7`) objects are Rust values. Shared ECMAScript modules
//! (`neo:assert` and `neo:test`) are constructed once per context from their
//! shared factories in `neojs-builtins` with native QuickJS host hooks.

#[cfg(feature = "neo-test")]
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
    /// The frozen public `neo:fs` object, when the feature is selected.
    #[cfg(feature = "neo-fs")]
    pub(super) fs: Option<Persistent<Object<'static>>>,
    /// The frozen public `neo:os` object, when the feature is selected.
    #[cfg(feature = "neo-os")]
    pub(super) os: Option<Persistent<Object<'static>>>,
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
    #[cfg(feature = "neo-fs")]
    let fs: Option<Object<'js>> = Some(super::fs::object(ctx)?);
    #[cfg(feature = "neo-os")]
    let os: Option<Object<'js>> = Some(super::os::object(ctx)?);
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
            feature = "neo-fs",
            feature = "neo-os",
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
        #[cfg(feature = "neo-fs")]
        fs: fs.map(|fs| Persistent::save(ctx, fs)),
        #[cfg(feature = "neo-os")]
        os: os.map(|os| Persistent::save(ctx, os)),
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
    host.set("print", Function::new(ctx.clone(), test_print)?)?;
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
fn test_print(text: Opt<Value<'_>>, is_error: Opt<Value<'_>>) -> Result<()> {
    let Some(text) = text.0.and_then(|value| value.into_string()) else {
        return Ok(());
    };
    let text = text.to_string()?;
    let is_error = is_error
        .0
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if is_error {
        eprint!("{text}");
    } else {
        print!("{text}");
    }
    Ok(())
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
        feature = "neo-fs",
        feature = "neo-os",
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
