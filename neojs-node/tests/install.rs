#![cfg(unix)]

use deno_core::JsRuntime;
use deno_core::v8;
use neojs_node::ChildProcessPolicy;
use neojs_node::NodeOptions;
use neojs_node::install;

#[test]
fn a_rejected_duplicate_install_preserves_the_existing_policy() {
    let mut js = JsRuntime::new(Default::default());
    let allowed = ChildProcessPolicy::new()
        .allow_program("/bin/echo")
        .expect("allow /bin/echo");
    install(&mut js, NodeOptions::new(allowed)).expect("first install");

    let denied = NodeOptions::new(ChildProcessPolicy::new());
    let error = install(&mut js, denied).expect_err("duplicate install must fail");
    assert!(error.contains("already registered"), "{error}");

    let context = js.main_context();
    v8::scope!(let scope, js.v8_isolate());
    let context = v8::Local::new(scope, context);
    let policy = context
        .get_slot::<NodeOptions>()
        .expect("the original policy slot is still present");
    let echo = std::fs::canonicalize("/bin/echo").expect("canonical /bin/echo");
    let true_path = std::fs::canonicalize("/bin/true").expect("canonical /bin/true");
    assert!(policy.child_process().allows(&echo));
    assert!(!policy.child_process().allows(&true_path));
}
