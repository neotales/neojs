#![cfg(all(
    feature = "neo-chars",
    feature = "neo-slices",
    feature = "neo-strings",
    feature = "neo-template"
))]

//! Live runtime CI drift test proving 1:1 parity between `neojs-dts` V8
//! declarations (`Profile::V8`) and the live V8 `Runtime::new()` built-in
//! globals and `neo:*` module exports.

use std::collections::BTreeSet;
use std::fs;

use neojs_core::Runtime;
use neojs_dts::{
    Profile, expected_exports, globals_dts, module_dts, parse_declared_exports, render_bundle,
};
use serde_json::Value;

#[derive(Debug)]
struct ObservedModule {
    specifier: String,
    ns_keys: Vec<String>,
    has_default: bool,
    reg_type: String,
    reg_keys: Vec<String>,
}

#[derive(Debug)]
struct ObservedRuntime {
    engine: String,
    console_keys: Vec<String>,
    console_methods: Vec<String>,
    list: Vec<String>,
    modules: Vec<ObservedModule>,
}

fn parse_observed(json_text: &str) -> ObservedRuntime {
    let root: Value = serde_json::from_str(json_text).expect("valid runtime JSON report");
    let engine = root["engine"].as_str().expect("engine string").to_string();
    let console_keys = root["consoleKeys"]
        .as_array()
        .expect("consoleKeys array")
        .iter()
        .map(|v| v.as_str().expect("console key string").to_string())
        .collect();
    let console_methods = root["consoleMethods"]
        .as_array()
        .expect("consoleMethods array")
        .iter()
        .map(|v| v.as_str().expect("console method string").to_string())
        .collect();
    let list = root["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|v| v.as_str().expect("specifier string").to_string())
        .collect();
    let modules = root["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .map(|m| ObservedModule {
            specifier: m["specifier"].as_str().expect("specifier").to_string(),
            ns_keys: m["nsKeys"]
                .as_array()
                .expect("nsKeys")
                .iter()
                .map(|v| v.as_str().expect("key").to_string())
                .collect(),
            has_default: m["hasDefault"].as_bool().expect("hasDefault"),
            reg_type: m["regType"].as_str().expect("regType").to_string(),
            reg_keys: m["regKeys"]
                .as_array()
                .expect("regKeys")
                .iter()
                .map(|v| v.as_str().expect("reg key").to_string())
                .collect(),
        })
        .collect();

    ObservedRuntime {
        engine,
        console_keys,
        console_methods,
        list,
        modules,
    }
}

#[test]
fn v8_runtime_exports_match_neojs_dts_one_to_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let report_path = dir.path().join("v8-drift.json");
    let entry_path = dir.path().join("inspect.js");

    let js = format!(
        r#"
import {{ writeTextFile }} from "neo:fs";

const reportPath = {report_path};
const list = Neo.getBuiltInModulesList();
const modules = [];

for (const specifier of list) {{
    const reg = Neo.getBuiltInModule(specifier);
    if (reg === undefined) {{
        throw new Error(`Neo.getBuiltInModule(${{specifier}}) returned undefined`);
    }}
    const ns = await import(specifier);
    const nsKeys = Object.keys(ns).sort();
    const hasDefault = Object.prototype.hasOwnProperty.call(ns, "default");
    const regKeys = Object.keys(reg).sort();
    modules.push({{
        specifier,
        nsKeys,
        hasDefault,
        regType: typeof reg,
        regKeys,
    }});
}}

const documentedConsoleMethods = ["log", "debug", "info", "warn", "error"];
const consoleMethods = documentedConsoleMethods.filter(
    (name) => typeof globalThis.console[name] === "function"
);

const report = {{
    engine: Neo.engine,
    consoleKeys: Object.keys(globalThis.console).sort(),
    consoleMethods,
    list: Array.from(list),
    modules,
}};

writeTextFile(reportPath, JSON.stringify(report));
"#,
        report_path = serde_json::to_string(report_path.to_str().expect("UTF-8 path"))
            .expect("serialize path"),
    );

    fs::write(&entry_path, js).expect("write inspect.js");
    let mut runtime = Runtime::new().expect("boot V8 Runtime");
    runtime.run_file(&entry_path).expect("run inspect.js");

    let raw = fs::read_to_string(&report_path).expect("read v8-drift.json");
    let observed = parse_observed(&raw);

    // 1. Neo.engine & console parity against Profile::V8 globals.d.ts.
    assert_eq!(observed.engine, Profile::V8.engine());
    let globals = globals_dts(Profile::V8);
    assert!(globals.contains(&format!("readonly engine: \"{}\";", Profile::V8.engine())));

    let expected_console = ["log", "debug", "info", "warn", "error"];
    assert_eq!(observed.console_methods, expected_console);
    for method in expected_console {
        assert!(
            observed.console_keys.iter().any(|k| k == method),
            "V8 globalThis.console missing own key `{method}` in {:?}",
            observed.console_keys
        );
        assert!(
            globals.contains(&format!("{method}(")),
            "V8 globals.d.ts missing console.{method}"
        );
    }

    // 2. Compare Neo.getBuiltInModulesList() against expected_exports and parse_declared_exports.
    let expected = expected_exports(Profile::V8);
    let bundle = render_bundle(Profile::V8);
    let declared = parse_declared_exports(&bundle);

    let expected_specs: Vec<&str> = expected.iter().map(|m| m.specifier).collect();
    let declared_specs: Vec<&str> = declared.iter().map(|m| m.specifier.as_str()).collect();
    let observed_specs: Vec<&str> = observed.list.iter().map(String::as_str).collect();

    assert_eq!(
        observed_specs, expected_specs,
        "Neo.getBuiltInModulesList() drifted from neojs_dts::expected_exports(Profile::V8)"
    );
    assert_eq!(
        declared_specs, expected_specs,
        "parse_declared_exports(render_bundle(Profile::V8)) drifted from expected_exports"
    );

    for spec in &observed_specs {
        assert!(
            globals.contains(&format!("\"{spec}\":")),
            "V8 globals.d.ts NeoBuiltInModuleMap missing `{spec}`"
        );
    }

    // 3. Per-module 1:1 export parity across live ESM namespace, expected_exports,
    //    bundle parse_declared_exports, and standalone module_dts.
    assert_eq!(observed.modules.len(), expected.len());
    for ((live, exp), decl) in observed
        .modules
        .iter()
        .zip(expected.iter())
        .zip(declared.iter())
    {
        assert_eq!(live.specifier, exp.specifier);
        assert_eq!(decl.specifier, exp.specifier);

        assert_eq!(
            live.has_default, exp.has_default,
            "{}: live default export ({}) != expected_exports ({})",
            live.specifier, live.has_default, exp.has_default
        );
        assert_eq!(
            decl.has_default, exp.has_default,
            "{}: declared default export ({}) != expected_exports ({})",
            live.specifier, decl.has_default, exp.has_default
        );

        let live_keys: Vec<&str> = live.ns_keys.iter().map(String::as_str).collect();
        let exp_keys = exp.runtime_keys();
        let decl_keys = decl.runtime_keys();

        assert_eq!(
            live_keys, exp_keys,
            "{}: live Object.keys(ns) drifted from neojs_dts::expected_exports(Profile::V8)",
            live.specifier
        );
        assert_eq!(
            live_keys, decl_keys,
            "{}: live Object.keys(ns) drifted from neojs_dts::parse_declared_exports",
            live.specifier
        );

        // Standalone module_dts must also parse identically.
        let standalone_src = module_dts(Profile::V8, exp.specifier)
            .unwrap_or_else(|| panic!("missing module_dts for {}", exp.specifier));
        let standalone_parsed = parse_declared_exports(standalone_src);
        assert_eq!(standalone_parsed.len(), 1);
        assert_eq!(
            standalone_parsed[0].runtime_keys(),
            live_keys,
            "{}: standalone module_dts runtime keys drifted",
            live.specifier
        );
        assert_eq!(standalone_parsed[0].has_default, live.has_default);

        // Check Neo.getBuiltInModule(specifier) shape matches declared NeoBuiltInModuleMap semantics.
        if live.specifier == "neo:assert" {
            assert_eq!(live.reg_type, "function");
            let reg_set: BTreeSet<&str> = live.reg_keys.iter().map(String::as_str).collect();
            for name in exp.exports {
                if *name != "assert" {
                    assert!(
                        reg_set.contains(name),
                        "neo:assert registry function missing property `{name}`"
                    );
                }
            }
        } else if live.specifier == "neo:console" {
            assert_eq!(live.reg_type, "object");
            let reg_set: BTreeSet<&str> = live.reg_keys.iter().map(String::as_str).collect();
            for name in exp.exports {
                assert!(
                    reg_set.contains(name),
                    "neo:console registry object missing method `{name}`"
                );
            }
        } else if live.specifier == "neo:uuid/v4" || live.specifier == "neo:uuid/v7" {
            assert_eq!(live.reg_type, "object", "{}", live.specifier);
            assert_eq!(
                live.reg_keys.iter().map(String::as_str).collect::<Vec<_>>(),
                exp_keys,
                "{}: Neo.getBuiltInModule keys drifted",
                live.specifier
            );
        } else {
            assert_eq!(live.reg_type, "object", "{}", live.specifier);
            let mut exp_named = exp.exports.to_vec();
            exp_named.sort_unstable();
            assert_eq!(
                live.reg_keys.iter().map(String::as_str).collect::<Vec<_>>(),
                exp_named,
                "{}: Neo.getBuiltInModule keys drifted",
                live.specifier
            );
        }
    }
}
