//! Native `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7` bindings for QuickJS.
//!
//! The RFC 9562 generation logic, fallible `getrandom` + `uuid::Builder`
//! handling, version 7 clock bounds, and stable error codes live in the
//! engine-neutral [`neojs_builtins::uuid`] module. This file creates one native
//! QuickJS function object each for `v4` and `v7` and shares those exact
//! function objects across `Neo.getBuiltInModule("neo:uuid")`,
//! `Neo.getBuiltInModule("neo:uuid/v4")`, `Neo.getBuiltInModule("neo:uuid/v7")`,
//! and all ES module exports.

use neojs_builtins::uuid as logic;
use rquickjs::{
    Ctx, Exception, Function, Object, Result, Value, function::Rest, module::ModuleDef,
    object::Property,
};

#[cfg(feature = "test-support")]
static FAIL_NEXT_UUID_RANDOM: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(super) fn fail_next_uuid_random() {
    FAIL_NEXT_UUID_RANDOM.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_uuid_random_failure() -> bool {
    FAIL_NEXT_UUID_RANDOM.swap(false, std::sync::atomic::Ordering::SeqCst)
}

#[cfg(feature = "test-support")]
fn fill_random_or_inject(bytes: &mut [u8]) -> std::result::Result<(), logic::UuidError> {
    if take_uuid_random_failure() {
        return Err(logic::UuidError::unexpected_random());
    }
    logic::fill_random(bytes)
}

/// Frozen registry objects for `neo:uuid`, `neo:uuid/v4`, and `neo:uuid/v7`,
/// sharing the same two native `v4` and `v7` function objects.
pub(super) struct UuidObjects<'js> {
    pub(super) uuid: Object<'js>,
    pub(super) v4: Object<'js>,
    pub(super) v7: Object<'js>,
}

/// Creates the frozen public registry objects for `neo:uuid` and its subpaths.
pub(super) fn objects<'js>(ctx: &Ctx<'js>) -> Result<UuidObjects<'js>> {
    let v4_fn = Function::new(ctx.clone(), v4)?
        .with_name("v4")?
        .with_length(0)?;
    let v7_fn = Function::new(ctx.clone(), v7)?
        .with_name("v7")?
        .with_length(0)?;

    let uuid = Object::new(ctx.clone())?;
    uuid.set("v4", v4_fn.clone())?;
    uuid.set("v7", v7_fn.clone())?;
    super::neo::freeze(ctx, &uuid)?;

    let v4 = Object::new(ctx.clone())?;
    v4.set("default", v4_fn.clone())?;
    v4.set("v4", v4_fn)?;
    super::neo::freeze(ctx, &v4)?;

    let v7 = Object::new(ctx.clone())?;
    v7.set("default", v7_fn.clone())?;
    v7.set("v7", v7_fn)?;
    super::neo::freeze(ctx, &v7)?;

    Ok(UuidObjects { uuid, v4, v7 })
}

fn v4<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    if !args.0.is_empty() {
        return Err(Exception::throw_type(&ctx, "v4 takes no arguments"));
    }
    #[cfg(feature = "test-support")]
    let result = logic::v4_with_entropy(fill_random_or_inject);
    #[cfg(not(feature = "test-support"))]
    let result = logic::v4();

    result.map_err(|error| throw_uuid_error(&ctx, error))
}

fn v7<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    if !args.0.is_empty() {
        return Err(Exception::throw_type(&ctx, "v7 takes no arguments"));
    }
    #[cfg(feature = "test-support")]
    let result = logic::v7_now_with_entropy(fill_random_or_inject);
    #[cfg(not(feature = "test-support"))]
    let result = logic::v7();

    result.map_err(|error| throw_uuid_error(&ctx, error))
}

fn throw_uuid_error<'js>(ctx: &Ctx<'js>, error: logic::UuidError) -> rquickjs::Error {
    let exception = match Exception::from_message(ctx.clone(), error.code().message()) {
        Ok(exception) => exception,
        Err(err) => return err,
    };
    if let Err(err) = exception.prop(
        "code",
        Property::from(error.code().as_str())
            .writable()
            .enumerable()
            .configurable(),
    ) {
        return err;
    }
    if let Some(cause) = error.diagnostic()
        && let Err(err) = exception.prop(
            "cause",
            Property::from(cause.as_str()).writable().configurable(),
        )
    {
        return err;
    }
    exception.throw()
}

fn export_from_state<'js>(
    ctx: &Ctx<'js>,
    exports: &rquickjs::module::Exports<'js>,
    names: &[&str],
    select: fn(&super::neo::NeoState) -> Option<&rquickjs::Persistent<Object<'static>>>,
) -> Result<()> {
    let state = ctx
        .userdata::<super::neo::NeoState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let Some(persistent) = select(&state) else {
        return Err(rquickjs::Error::Unknown);
    };
    let object = persistent.clone().restore(ctx)?;
    for name in names {
        let value: Value<'js> = object.get(*name)?;
        exports.export(*name, value)?;
    }
    Ok(())
}

/// The native QuickJS module served for `neo:uuid`.
pub(super) struct UuidModule;

impl ModuleDef for UuidModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        export_from_state(ctx, exports, logic::EXPORTS, |state| state.uuid.as_ref())
    }
}

/// The native QuickJS module served for `neo:uuid/v4`.
pub(super) struct UuidV4Module;

impl ModuleDef for UuidV4Module {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::V4_EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        export_from_state(ctx, exports, logic::V4_EXPORTS, |state| {
            state.uuid_v4.as_ref()
        })
    }
}

/// The native QuickJS module served for `neo:uuid/v7`.
pub(super) struct UuidV7Module;

impl ModuleDef for UuidV7Module {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in logic::V7_EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        export_from_state(ctx, exports, logic::V7_EXPORTS, |state| {
            state.uuid_v7.as_ref()
        })
    }
}
