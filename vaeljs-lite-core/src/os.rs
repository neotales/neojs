//! Native `vael:os` binding for the QuickJS runtime.
//!
//! The build-target constants come from the engine-neutral `vaeljs_builtins::os`
//! module. This file creates the frozen QuickJS registry object and exports the
//! same constants from a native QuickJS module.

use rquickjs::{Ctx, Object, Result, Value, module::ModuleDef};
use vaeljs_builtins::os as logic;

/// Creates the frozen public module object.
///
/// The object is created once per context. The registry and the native module
/// both expose the same `platform` and `arch` string constants.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("platform", logic::platform())?;
    object.set("arch", logic::arch())?;
    super::vael::freeze(ctx, &object)?;
    Ok(object)
}

/// The native QuickJS module served for `vael:os`.
pub(super) struct OsModule;

impl ModuleDef for OsModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(os) = state.os.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = os.clone().restore(ctx)?;
        for name in logic::EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}
