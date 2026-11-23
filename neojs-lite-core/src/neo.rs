//! Native `Neo` introspection object for the QuickJS runtime.
//!
//! The registry, the module list, and the `neo:chars` object are Rust values.
//! No part of the surface is installed by JavaScript.

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
    /// The frozen public `neo:chars` object, when the feature is selected.
    #[cfg(feature = "neo-chars")]
    pub(super) chars: Option<Persistent<Object<'static>>>,
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
pub(super) fn install<'js>(ctx: &Ctx<'js>) -> Result<()> {
    #[cfg(feature = "neo-chars")]
    let chars: Option<Object<'js>> = Some(super::chars::object(ctx)?);

    let list = Array::new(ctx.clone())?;
    #[cfg(feature = "neo-chars")]
    if chars.is_some() {
        list.set(0, neojs_builtins::chars::SPECIFIER)?;
    }
    freeze(ctx, list.as_object())?;

    ctx.store_userdata(NeoState {
        #[cfg(feature = "neo-chars")]
        chars: chars.map(|chars| Persistent::save(ctx, chars)),
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
    #[cfg(feature = "neo-chars")]
    if name == neojs_builtins::chars::SPECIFIER {
        let Some(chars) = state.chars.as_ref() else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        return Ok(chars.clone().restore(&ctx)?.into_value());
    }
    #[cfg(not(feature = "neo-chars"))]
    let _ = state;
    let _ = name;
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
