#![cfg(all(
    feature = "neo-assert",
    feature = "neo-chars",
    feature = "neo-crypto",
    feature = "neo-fs",
    feature = "neo-os",
    feature = "neo-ssh",
    feature = "neo-strings",
    feature = "neo-test",
    feature = "neo-uuid"
))]

//! Live runtime CI drift test proving 1:1 parity between `neojs-dts` Lite
//! declarations (`Profile::Lite`) and the live QuickJS `Runtime::new()`
//! built-in globals and `neo:*` module exports.

use std::collections::BTreeSet;
use std::fs;

use neojs_dts::{
    Profile, expected_exports, globals_dts, module_dts, parse_declared_exports, render_bundle,
};
use neojs_lite_core::Runtime;

#[derive(Debug)]
struct ObservedModule {
    specifier: String,
    has_default: bool,
    ns_keys: Vec<String>,
    reg_type: String,
    reg_keys: Vec<String>,
}

#[derive(Debug)]
struct ObservedRuntime {
    engine: String,
    console_keys: Vec<String>,
    list: Vec<String>,
    modules: Vec<ObservedModule>,
}

fn parse_csv(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(',').map(str::to_string).collect()
    }
}

fn parse_report(text: &str) -> ObservedRuntime {
    let mut engine = None;
    let mut console_keys = None;
    let mut list = None;
    let mut modules = Vec::new();

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ENGINE\t") {
            engine = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("CONSOLE\t") {
            console_keys = Some(parse_csv(rest));
        } else if let Some(rest) = line.strip_prefix("LIST\t") {
            list = Some(parse_csv(rest));
        } else if let Some(rest) = line.strip_prefix("MODULE\t") {
            let parts: Vec<&str> = rest.split('\t').collect();
            assert_eq!(parts.len(), 5, "unexpected MODULE line: {line:?}");
            let has_default = match parts[1] {
                "true" => true,
                "false" => false,
                other => panic!("unexpected has_default value: {other:?}"),
            };
            modules.push(ObservedModule {
                specifier: parts[0].to_string(),
                has_default,
                ns_keys: parse_csv(parts[2]),
                reg_type: parts[3].to_string(),
                reg_keys: parse_csv(parts[4]),
            });
        } else {
            panic!("unexpected report line: {line:?}");
        }
    }

    ObservedRuntime {
        engine: engine.expect("ENGINE line"),
        console_keys: console_keys.expect("CONSOLE line"),
        list: list.expect("LIST line"),
        modules,
    }
}

#[test]
fn lite_runtime_exports_match_neojs_dts_one_to_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let report_path = dir.path().join("lite-drift.tsv");
    let report_path_str = report_path.to_str().expect("UTF-8 path");
    let escaped_path = report_path_str.replace('\\', "\\\\").replace('"', "\\\"");

    let js = format!(
        r#"
const {{ writeTextFile }} = Neo.getBuiltInModule("neo:fs");
const list = Neo.getBuiltInModulesList();
const lines = [
    `ENGINE\t${{Neo.engine}}`,
    `CONSOLE\t${{Object.keys(globalThis.console).sort().join(",")}}`,
    `LIST\t${{Array.from(list).join(",")}}`,
];

for (const name of Object.keys(globalThis.console)) {{
    if (typeof globalThis.console[name] !== "function") {{
        throw new Error(`console.${{name}} is not a function`);
    }}
}}

for (const specifier of list) {{
    const reg = Neo.getBuiltInModule(specifier);
    if (reg === undefined) {{
        throw new Error(`Neo.getBuiltInModule(${{specifier}}) returned undefined`);
    }}
    const ns = await import(specifier);
    const nsKeys = Object.keys(ns).sort();
    const hasDefault = Object.prototype.hasOwnProperty.call(ns, "default");
    const regKeys = Object.keys(reg).sort();

    // Verify identity between registry object and ESM namespace exports.
    for (const key of nsKeys) {{
        if (key === "default") {{
            if (specifier === "neo:assert") {{
                if (ns.default !== reg) throw new Error(`${{specifier}} default != reg`);
            }} else if (specifier === "neo:test") {{
                if (ns.default !== reg.test) throw new Error(`${{specifier}} default != reg.test`);
            }} else if (specifier === "neo:uuid/v4") {{
                if (ns.default !== reg.v4) throw new Error(`${{specifier}} default != reg.v4`);
            }} else if (specifier === "neo:uuid/v7") {{
                if (ns.default !== reg.v7) throw new Error(`${{specifier}} default != reg.v7`);
            }}
        }} else if (specifier === "neo:assert" && key === "assert") {{
            if (ns.assert !== reg) throw new Error("neo:assert.assert != reg");
        }} else if (ns[key] !== reg[key]) {{
            throw new Error(`${{specifier}}.${{key}} differs between namespace and registry`);
        }}
    }}

    lines.push(
        `MODULE\t${{specifier}}\t${{hasDefault}}\t${{nsKeys.join(",")}}\t${{typeof reg}}\t${{regKeys.join(",")}}`
    );
}}

writeTextFile("{escaped_path}", lines.join("\n"));
"#
    );

    let mut runtime = Runtime::new().expect("boot QuickJS Runtime");
    runtime
        .run_source("lite-drift.js", &js)
        .expect("run lite-drift.js");

    let raw = fs::read_to_string(&report_path).expect("read lite-drift.tsv");
    let observed = parse_report(&raw);

    // 1. Neo.engine & console parity against Profile::Lite globals.d.ts.
    assert_eq!(observed.engine, Profile::Lite.engine());
    let globals = globals_dts(Profile::Lite);
    assert!(globals.contains(&format!("readonly engine: \"{}\";", Profile::Lite.engine())));

    assert_eq!(
        observed.console_keys,
        vec!["error", "log"],
        "QuickJS console keys drifted from Profile::Lite"
    );
    assert!(globals.contains("log("));
    assert!(globals.contains("error("));
    for absent in ["debug(", "info(", "warn("] {
        assert!(
            !globals.contains(absent),
            "Lite globals.d.ts must not declare console.{absent}"
        );
    }

    // 2. Compare Neo.getBuiltInModulesList() against expected_exports and parse_declared_exports.
    let expected = expected_exports(Profile::Lite);
    let bundle = render_bundle(Profile::Lite);
    let declared = parse_declared_exports(&bundle);

    let expected_specs: Vec<&str> = expected.iter().map(|m| m.specifier).collect();
    let declared_specs: Vec<&str> = declared.iter().map(|m| m.specifier.as_str()).collect();
    let observed_specs: Vec<&str> = observed.list.iter().map(String::as_str).collect();

    assert_eq!(
        observed_specs, expected_specs,
        "Neo.getBuiltInModulesList() drifted from neojs_dts::expected_exports(Profile::Lite)"
    );
    assert_eq!(
        declared_specs, expected_specs,
        "parse_declared_exports(render_bundle(Profile::Lite)) drifted from expected_exports"
    );

    for spec in &observed_specs {
        assert!(
            globals.contains(&format!("\"{spec}\":")),
            "Lite globals.d.ts NeoBuiltInModuleMap missing `{spec}`"
        );
    }

    // 3. Per-module 1:1 export parity across live ESM namespace, registry,
    //    expected_exports, bundle parse_declared_exports, and standalone module_dts.
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
            "{}: live Object.keys(ns) drifted from neojs_dts::expected_exports(Profile::Lite)",
            live.specifier
        );
        assert_eq!(
            live_keys, decl_keys,
            "{}: live Object.keys(ns) drifted from neojs_dts::parse_declared_exports",
            live.specifier
        );

        let standalone_src = module_dts(Profile::Lite, exp.specifier)
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
