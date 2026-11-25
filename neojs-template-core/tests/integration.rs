//! Integration tests for `neojs-template-core`.

use neojs_template_core::{
    CompiledTemplate, TemplateError, TemplateOptions, TemplateSet, escape_html,
    json_to_template_value, render, sprig_call,
};
use neotales_gotmpl::Value;
use serde_json::json;

#[test]
fn plain_go_templates_control_flow_and_pipelines() {
    let opts = TemplateOptions::default();

    assert_eq!(
        render("Hello, {{ .name }}!", &json!({ "name": "Neo" }), &opts).unwrap(),
        "Hello, Neo!"
    );

    assert_eq!(
        render(
            "{{ if .ok }}yes{{ else }}no{{ end }}|{{ if .no }}yes{{ else }}no{{ end }}",
            &json!({ "ok": true, "no": false }),
            &opts
        )
        .unwrap(),
        "yes|no"
    );

    assert_eq!(
        render(
            "{{ range $i, $v := .items }}{{ $i }}:{{ $v }};{{ else }}empty{{ end }}",
            &json!({ "items": ["a", "b", "c"] }),
            &opts
        )
        .unwrap(),
        "0:a;1:b;2:c;"
    );

    assert_eq!(
        render(
            "{{ with .user }}hi {{ .name }}{{ else }}anon{{ end }}|{{ with .missing }}x{{ else }}anon{{ end }}",
            &json!({ "user": { "name": "Ada" }, "missing": null }),
            &opts
        )
        .unwrap(),
        "hi Ada|anon"
    );

    assert_eq!(
        render(
            "{{ len .items }}|{{ index .items 1 }}|{{ .title | trim | upper }}",
            &json!({ "items": ["x", "y"], "title": "  hello  " }),
            &opts
        )
        .unwrap(),
        "2|y|HELLO"
    );
}

#[test]
fn sprig_helpers_work_across_categories() {
    let opts = TemplateOptions::default();
    let data = json!({
        "raw": "  hello world  ",
        "empty": "",
        "csv": "a,b,c",
        "obj": { "z": 1, "a": "ok" }
    });

    assert_eq!(
        render("{{ .raw | trim | upper }}", &data, &opts).unwrap(),
        "HELLO WORLD"
    );
    assert_eq!(
        render("{{ \"HELLO\" | lower }}", &data, &opts).unwrap(),
        "hello"
    );
    assert_eq!(
        render("{{ \"hello world\" | title }}", &data, &opts).unwrap(),
        "Hello World"
    );
    assert_eq!(
        render("{{ default \"fallback\" .empty }}", &data, &opts).unwrap(),
        "fallback"
    );
    assert_eq!(
        render("{{ \"foo-bar-baz\" | replace \"-\" \"_\" }}", &data, &opts).unwrap(),
        "foo_bar_baz"
    );
    assert_eq!(
        render("{{ splitList \",\" .csv | join \"|\" }}", &data, &opts).unwrap(),
        "a|b|c"
    );
    assert_eq!(
        render("{{ add 2 3 }}|{{ mul 4 5 }}", &data, &opts).unwrap(),
        "5|20"
    );
    assert_eq!(
        render(
            "{{ $d := dict \"b\" 2 \"a\" 1 }}{{ get $d \"a\" }}|{{ list \"x\" \"y\" | join \":\" }}",
            &data,
            &opts
        )
        .unwrap(),
        "1|x:y"
    );
    assert_eq!(
        render("{{ toJson .obj }}", &data, &opts).unwrap(),
        r#"{"a":"ok","z":1}"#
    );
    assert_eq!(
        render(
            r#"{{ semver "1.2.3" | (semver "1.3.3").Compare }}"#,
            &json!({}),
            &opts
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn sprig_false_refuses_sprig_helpers_but_keeps_builtins_and_html_helpers() {
    let opts = TemplateOptions {
        sprig: false,
        ..TemplateOptions::default()
    };

    let err = render("{{ .name | upper }}", &json!({ "name": "neo" }), &opts).unwrap_err();
    assert!(
        matches!(err, TemplateError::Render(ref msg) if msg.contains("upper")),
        "expected Render error mentioning upper, got {err:?}"
    );

    assert_eq!(
        render(
            "{{ len .items }}|{{ escapeHtml .raw }}|{{ safeHtml .raw }}",
            &json!({ "items": [1, 2], "raw": "<b>x</b>" }),
            &opts
        )
        .unwrap(),
        "2|&lt;b&gt;x&lt;/b&gt;|<b>x</b>"
    );
}

#[test]
fn hermetic_true_refuses_nonhermetic_helpers_and_allows_deterministic_ones() {
    let opts = TemplateOptions {
        hermetic: true,
        ..TemplateOptions::default()
    };

    for non_hermetic in [
        "{{ now }}",
        "{{ ago \"2025-01-01T00:00:00Z\" }}",
        "{{ env \"PATH\" }}",
    ] {
        let err = render(non_hermetic, &json!({}), &opts).unwrap_err();
        assert!(
            matches!(err, TemplateError::Render(_)),
            "expected Render error for {non_hermetic}, got {err:?}"
        );
    }

    assert_eq!(
        render(
            "{{ .name | upper }}:{{ add 10 20 }}",
            &json!({ "name": "neo" }),
            &opts
        )
        .unwrap(),
        "NEO:30"
    );
}

#[test]
fn compiled_template_reuse_across_inputs() {
    let compiled = CompiledTemplate::compile(
        "User {{ .name }} (role={{ default \"guest\" .role }})",
        &TemplateOptions::default(),
    )
    .unwrap();

    assert_eq!(
        compiled
            .render(&json!({ "name": "Alice", "role": "admin" }))
            .unwrap(),
        "User Alice (role=admin)"
    );
    assert_eq!(
        compiled
            .render(&json!({ "name": "Bob", "role": "" }))
            .unwrap(),
        "User Bob (role=guest)"
    );
}

#[test]
fn template_set_layouts_partials_and_block_overrides() {
    let mut set = TemplateSet::new(TemplateOptions::default());
    set.add(
        "partials",
        "{{define \"badge\"}}[{{. | upper}}]{{end}}{{define \"footer\"}}<footer>v1</footer>{{end}}",
    )
    .unwrap()
    .add(
        "layout",
        "<html><head><title>{{block \"title\" .}}Default Title{{end}}</title></head><body>{{block \"content\" .}}Empty{{end}}{{template \"footer\" .}}</body></html>",
    )
    .unwrap()
    .add(
        "home",
        "{{define \"title\"}}Home{{end}}{{define \"content\"}}<h1>Hi {{.user}} {{template \"badge\" .role}}</h1>{{end}}{{template \"layout\" .}}",
    )
    .unwrap()
    .add(
        "about",
        "{{define \"content\"}}<p>About {{.org}}</p>{{end}}{{template \"layout\" .}}",
    )
    .unwrap()
    .add("standalone_partial", "<span>{{.item}}</span>")
    .unwrap()
    .add("use_partial", "Item: {{template \"standalone_partial\" .}}")
    .unwrap();

    assert_eq!(
        set.names(),
        vec![
            "about",
            "home",
            "layout",
            "partials",
            "standalone_partial",
            "use_partial"
        ]
    );

    assert_eq!(
        set.render("home", &json!({ "user": "Ada", "role": "dev" }))
            .unwrap(),
        "<html><head><title>Home</title></head><body><h1>Hi Ada [DEV]</h1><footer>v1</footer></body></html>"
    );

    // Sibling page `about` omits `title`, so `layout`'s block default is used
    // rather than leaking `home`'s title.
    assert_eq!(
        set.render("about", &json!({ "org": "NeoTales" })).unwrap(),
        "<html><head><title>Default Title</title></head><body><p>About NeoTales</p><footer>v1</footer></body></html>"
    );

    // Rendering `layout` directly uses both block defaults.
    assert_eq!(
        set.render("layout", &json!({})).unwrap(),
        "<html><head><title>Default Title</title></head><body>Empty<footer>v1</footer></body></html>"
    );

    assert_eq!(
        set.render("use_partial", &json!({ "item": "widget" }))
            .unwrap(),
        "Item: <span>widget</span>"
    );

    let missing_err = set.render("does_not_exist", &json!({})).unwrap_err();
    assert_eq!(
        missing_err,
        TemplateError::TemplateNotFound("does_not_exist".to_owned())
    );
}

#[test]
fn template_set_rejects_invalid_names_and_missing_partials() {
    let mut set = TemplateSet::new(TemplateOptions::default());

    assert!(matches!(
        set.add("", "hi"),
        Err(TemplateError::InvalidName(_))
    ));
    assert!(matches!(
        set.add("   ", "hi"),
        Err(TemplateError::InvalidName(_))
    ));
    assert!(matches!(
        set.add("bad\nname", "hi"),
        Err(TemplateError::InvalidName(_))
    ));

    set.add("caller", "{{template \"missing_partial\" .}}")
        .unwrap();
    let err = set.render("caller", &json!({})).unwrap_err();
    assert_eq!(
        err,
        TemplateError::TemplateNotFound("missing_partial".to_owned())
    );
}

#[test]
fn html_escape_helpers_auto_escaping_and_safe_html() {
    assert_eq!(
        escape_html(r#"<script attr="x" single='y'>a & b</script>"#),
        "&lt;script attr=&quot;x&quot; single=&#39;y&#39;&gt;a &amp; b&lt;/script&gt;"
    );

    // Manual helpers work when html_escape is false.
    let plain_opts = TemplateOptions::default();
    assert_eq!(
        render(
            "{{ .x | escapeHtml }}|{{ htmlEscape .x }}|{{ .x | safeHtml }}",
            &json!({ "x": "<b>hi</b>" }),
            &plain_opts
        )
        .unwrap(),
        "&lt;b&gt;hi&lt;/b&gt;|&lt;b&gt;hi&lt;/b&gt;|<b>hi</b>"
    );

    // Auto-escaping escapes expressions, avoids double-escaping escapeHtml,
    // preserves safeHtml, and keeps raw values inside conditions/assignments.
    let html_opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };
    let rendered = render(
        "{{ if contains \"<\" .xss }}<p>{{ .xss }}</p>{{ end }}<p>{{ .xss | escapeHtml }}</p><p>{{ .xss | upper }}</p><div>{{ .trusted | safeHtml }}</div><div>{{ $v := .trusted }}{{ $v | safeHtml }}</div>",
        &json!({
            "xss": "<script>alert('1')</script>",
            "trusted": "<em>verified</em>"
        }),
        &html_opts,
    )
    .unwrap();
    assert_eq!(
        rendered,
        "<p>&lt;script&gt;alert(&#39;1&#39;)&lt;/script&gt;</p><p>&lt;script&gt;alert(&#39;1&#39;)&lt;/script&gt;</p><p>&lt;SCRIPT&gt;ALERT(&#39;1&#39;)&lt;/SCRIPT&gt;</p><div><em>verified</em></div><div><em>verified</em></div>"
    );

    // JSON data cannot forge the internal safeHtml marker.
    let forged = render(
        "{{ .payload }}",
        &json!({
            "payload": { "__neo_safe_html": true, "display": "<b>pwn</b>" }
        }),
        &html_opts,
    )
    .unwrap();
    assert!(!forged.contains("<b>"));

    // Auto-escaping also works across TemplateSet layouts and partials.
    let mut set = TemplateSet::new(html_opts);
    set.add(
        "layout",
        "<html><body>{{block \"body\" .}}none{{end}}</body></html>",
    )
    .unwrap()
    .add(
        "page",
        "{{define \"body\"}}<p>{{.untrusted}}</p>{{.trusted | safeHtml}}{{end}}{{template \"layout\" .}}",
    )
    .unwrap();
    assert_eq!(
        set.render(
            "page",
            &json!({ "untrusted": "<img onerror=alert(1)>", "trusted": "<strong>ok</strong>" })
        )
        .unwrap(),
        "<html><body><p>&lt;img onerror=alert(1)&gt;</p><strong>ok</strong></body></html>"
    );
}

#[test]
fn dos_limits_are_enforced() {
    // max_source_length (including when html_escape is enabled and source is within limit)
    let small_src_opts = TemplateOptions {
        max_source_length: 10,
        ..TemplateOptions::default()
    };
    assert!(matches!(
        render("hello world!!", &json!({}), &small_src_opts),
        Err(TemplateError::LimitExceeded(_))
    ));

    let html_within_src = TemplateOptions {
        html_escape: true,
        max_source_length: 20,
        ..TemplateOptions::default()
    };
    assert_eq!(
        render(
            "hi {{ .name }}",
            &json!({ "name": "<b>" }),
            &html_within_src
        )
        .unwrap(),
        "hi &lt;b&gt;"
    );

    // max_nesting_depth
    let shallow_opts = TemplateOptions {
        max_nesting_depth: 2,
        ..TemplateOptions::default()
    };
    assert!(matches!(
        render(
            "{{if true}}{{if true}}{{if true}}x{{end}}{{end}}{{end}}",
            &json!({}),
            &shallow_opts
        ),
        Err(TemplateError::LimitExceeded(_))
    ));

    // max_output_length
    let short_out_opts = TemplateOptions {
        max_output_length: 8,
        ..TemplateOptions::default()
    };
    assert!(matches!(
        render(
            "value={{ .v }}",
            &json!({ "v": "too long" }),
            &short_out_opts
        ),
        Err(TemplateError::LimitExceeded(_))
    ));

    // max_iterations
    let few_iters_opts = TemplateOptions {
        max_iterations: 2,
        ..TemplateOptions::default()
    };
    assert!(matches!(
        render(
            "{{ range .items }}{{ . }}{{ end }}",
            &json!({ "items": [1, 2, 3] }),
            &few_iters_opts
        ),
        Err(TemplateError::LimitExceeded(_))
    ));

    // max_function_calls
    let few_calls_opts = TemplateOptions {
        max_function_calls: 1,
        ..TemplateOptions::default()
    };
    assert!(matches!(
        render(
            "{{ upper \"a\" }}{{ upper \"b\" }}",
            &json!({}),
            &few_calls_opts
        ),
        Err(TemplateError::LimitExceeded(_))
    ));
}

#[test]
fn invalid_syntax_and_invalid_data_errors() {
    let opts = TemplateOptions::default();

    for bad_syntax in [
        "{{ .name",
        "{{ /* unclosed comment",
        "{{ }}",
        "{{ if .a }}",
        "{{ else }}",
        "{{ block \"x\" }}body{{ end }}",
        "{{ define \"a\" }}1{{ end }}{{ define \"a\" }}2{{ end }}",
    ] {
        assert!(
            matches!(
                render(bad_syntax, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected Parse error for {bad_syntax:?}"
        );
    }

    for bad_data in [json!(null), json!(42), json!("text"), json!([1, 2])] {
        assert!(
            matches!(
                render("hello", &bad_data, &opts),
                Err(TemplateError::InvalidData(_))
            ),
            "expected InvalidData for {bad_data:?}"
        );
    }
}

#[test]
fn json_conversion_and_sprig_call_adapter() {
    assert_eq!(json_to_template_value(&json!(null)), Value::Null);
    assert_eq!(json_to_template_value(&json!(false)), Value::Bool(false));
    assert_eq!(json_to_template_value(&json!(-7)), Value::Number(-7.0));
    assert_eq!(json_to_template_value(&json!(42u64)), Value::Number(42.0));
    assert_eq!(json_to_template_value(&json!(3.25)), Value::Float(3.25));
    assert_eq!(
        json_to_template_value(&json!("neo")),
        Value::String("neo".to_owned())
    );
    assert_eq!(
        json_to_template_value(&json!([1, "a"])),
        Value::List(vec![Value::Number(1.0), Value::String("a".to_owned())])
    );

    // `takes_rendered_strings("upper")` is true: numbers are rendered to strings.
    assert_eq!(
        sprig_call("upper", &[Value::Number(123.0)]).unwrap(),
        Value::String("123".to_owned())
    );

    // `takes_rendered_strings("join")` is false: list structure is preserved.
    assert_eq!(
        sprig_call(
            "join",
            &[
                Value::String("-".to_owned()),
                Value::List(vec![Value::String("a".to_owned()), Value::Number(2.0)])
            ]
        )
        .unwrap(),
        Value::String("a-2".to_owned())
    );

    // Unknown helper returns an error.
    assert!(sprig_call("definitelyNotAHelper", &[]).is_err());

    // Large u64, nested object conversion, and NaN/Inf handling through sprig_call.
    let nested = json_to_template_value(&json!({
        "big": u64::MAX,
        "min": i64::MIN,
        "nested": { "arr": [1, true, { "k": "v" }] }
    }));
    assert!(matches!(nested, Value::Map(_)));

    assert_eq!(
        sprig_call("toString", &[Value::Number(f64::NAN)]).unwrap(),
        Value::String(Value::Number(f64::NAN).render())
    );
    assert_eq!(
        sprig_call("toString", &[Value::Float(f64::NAN)]).unwrap(),
        Value::String(Value::Float(f64::NAN).render())
    );
    assert_eq!(
        sprig_call("toString", &[Value::Float(f64::INFINITY)]).unwrap(),
        Value::String("+Inf".to_owned())
    );
    assert_eq!(
        sprig_call("toString", &[Value::Float(f64::NEG_INFINITY)]).unwrap(),
        Value::String("-Inf".to_owned())
    );
}

#[test]
fn template_set_handles_nested_control_flow_self_define_cycles_and_bundles() {
    // 1. Nested if/range/with inside define and block.
    let mut set = TemplateSet::new(TemplateOptions::default());
    set.add(
        "nested",
        "{{define \"list_item\"}}{{if .active}}<li>{{with .label}}{{.}}{{end}}</li>{{end}}{{end}}{{block \"wrapper\" .}}<ul>{{range .items}}{{template \"list_item\" .}}{{end}}</ul>{{end}}",
    )
    .unwrap();
    assert_eq!(
        set.render(
            "nested",
            &json!({
                "items": [
                    { "active": true, "label": "One" },
                    { "active": false, "label": "Two" },
                    { "active": true, "label": "Three" }
                ]
            })
        )
        .unwrap(),
        "<ul><li>One</li><li>Three</li></ul>"
    );

    // 2. Entry's own {{define "<name>"}} renders as root body without duplicating itself.
    let mut self_def = TemplateSet::new(TemplateOptions::default());
    self_def
        .add("foo", "{{define \"foo\"}}defined:{{.x}}{{end}}outer")
        .unwrap();
    assert_eq!(
        self_def.render("foo", &json!({ "x": "ok" })).unwrap(),
        "defined:ok"
    );

    // 3. Static cycle rejection across templates in TemplateSet.
    let mut cyclic = TemplateSet::new(TemplateOptions::default());
    cyclic.add("a", "{{template \"b\" .}}").unwrap();
    cyclic.add("b", "{{template \"a\" .}}").unwrap();
    assert!(matches!(
        cyclic.render("a", &json!({})),
        Err(TemplateError::Parse(_))
    ));

    // 4. Partial-bundle entry preserving its own block_defaults.
    let mut bundled = TemplateSet::new(TemplateOptions::default());
    bundled
        .add(
            "bundle",
            "{{define \"card\"}}<div class=\"card\">{{block \"card_footer\" .}}default-footer{{end}}</div>{{end}}",
        )
        .unwrap()
        .add("page", "{{template \"card\" .}}")
        .unwrap()
        .add(
            "custom_page",
            "{{define \"card_footer\"}}custom-footer{{end}}{{template \"card\" .}}",
        )
        .unwrap();
    assert_eq!(
        bundled.render("page", &json!({})).unwrap(),
        "<div class=\"card\">default-footer</div>"
    );
    assert_eq!(
        bundled.render("custom_page", &json!({})).unwrap(),
        "<div class=\"card\">custom-footer</div>"
    );

    // 5. Ambiguous cross-entry define rejection.
    let mut ambiguous = TemplateSet::new(TemplateOptions::default());
    ambiguous
        .add("bundle_a", "{{define \"shared\"}}A{{end}}")
        .unwrap()
        .add("bundle_b", "{{define \"shared\"}}B{{end}}")
        .unwrap()
        .add("caller", "{{template \"shared\" .}}")
        .unwrap();
    assert!(matches!(
        ambiguous.render("caller", &json!({})),
        Err(TemplateError::Parse(_))
    ));
}

#[test]
fn template_set_enforces_max_source_length_on_composed_render() {
    let opts = TemplateOptions {
        max_source_length: 40,
        ..TemplateOptions::default()
    };
    let mut set = TemplateSet::new(opts);
    // Each entry individually is <= 40 bytes, but combined exceeds 40 bytes.
    set.add("part", "012345678901234567890123456789").unwrap();
    set.add("main", "{{template \"part\" .}}abcdefgh").unwrap();
    assert!(matches!(
        set.render("main", &json!({})),
        Err(TemplateError::LimitExceeded(_))
    ));
}

#[test]
fn template_escape_sequences_in_define_and_template_names() {
    let mut set = TemplateSet::new(TemplateOptions::default());
    // \x41, \u0042, \103 all decode to "ABC".
    set.add(
        "p",
        "{{define \"\\x41\\u0042\\103\"}}decoded{{end}}{{template \"ABC\" .}}",
    )
    .unwrap();
    assert_eq!(set.render("p", &json!({})).unwrap(), "decoded");

    // Standalone block with \x41 name.
    assert_eq!(
        render(
            "{{block \"\\x41\" .}}ok{{end}}",
            &json!({}),
            &TemplateOptions::default()
        )
        .unwrap(),
        "ok"
    );

    // Invalid escape sequence is rejected.
    assert!(matches!(
        render(
            "{{block \"\\q\" .}}no{{end}}",
            &json!({}),
            &TemplateOptions::default()
        ),
        Err(TemplateError::Parse(_))
    ));
}

#[test]
fn template_set_preserves_whitespace_boundary_across_removed_define() {
    let mut set = TemplateSet::new(TemplateOptions::default());
    set.add("page", "A  {{define \"x\"}}X{{end}}  {{- .name}}")
        .unwrap();
    assert_eq!(
        set.render("page", &json!({ "name": "Neo" })).unwrap(),
        "A  Neo"
    );
}
