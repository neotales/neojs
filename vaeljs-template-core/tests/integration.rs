//! Integration tests for `vaeljs-template-core`.

use neotales_gotmpl::Value;
use serde_json::json;
use vaeljs_template_core::{
    CompiledTemplate, TemplateError, TemplateOptions, TemplateSet, escape_html,
    json_to_template_value, render, sprig_call,
};

#[test]
fn plain_go_templates_control_flow_and_pipelines() {
    let opts = TemplateOptions::default();

    assert_eq!(
        render("Hello, {{ .name }}!", &json!({ "name": "Vael" }), &opts).unwrap(),
        "Hello, Vael!"
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
            &json!({ "name": "vael" }),
            &opts
        )
        .unwrap(),
        "VAEL:30"
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
            "payload": { "__vael_safe_html": true, "display": "<b>pwn</b>" }
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
        set.render("page", &json!({ "name": "Vael" })).unwrap(),
        "A  Vael"
    );
}

#[test]
fn html_escape_control_keywords_and_variable_reassignments_work() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };
    let plain_opts = TemplateOptions::default();

    // Variable declarations ($x := ..., $k, $v := ...) work and preserve raw types before escaping output.
    let vars = "{{$x := \"<b>init</b>\"}}{{$x}}|{{range $k, $v := .list}}{{$k}}={{$v}};{{end}}";
    assert_eq!(
        render(vars, &json!({ "list": ["<x>", "y"] }), &opts).unwrap(),
        "&lt;b&gt;init&lt;/b&gt;|0=&lt;x&gt;;1=y;"
    );

    // Non-assignment expression containing '=' inside pipeline string literal still escapes.
    assert_eq!(
        render("{{$x := \"\"}}{{$x | default \"a=<b\"}}", &json!({}), &opts).unwrap(),
        "a=&lt;b"
    );

    // Control keywords / reassignments not yet supported by neotales_gotmpl are left
    // untouched by rewrite_html_escape (neither wrapped nor leaking __vael_auto_escape).
    for tmpl in [
        "{{if .a}}A{{else if .b}}B{{end}}",
        "{{with .a}}A{{else with .b}}B{{end}}",
        "{{range .items}}{{break}}{{end}}",
        "{{range .items}}{{continue}}{{end}}",
        "{{$x := 1}}{{$x = 2}}{{$x}}",
    ] {
        let plain_res = render(
            tmpl,
            &json!({ "a": false, "b": true, "items": [1] }),
            &plain_opts,
        );
        let html_res = render(tmpl, &json!({ "a": false, "b": true, "items": [1] }), &opts);
        match (plain_res, html_res) {
            (Ok(plain_out), Ok(html_out)) => assert_eq!(plain_out, html_out, "mismatch for {tmpl}"),
            (Err(plain_err), Err(html_err)) => {
                assert_eq!(
                    std::mem::discriminant(&plain_err),
                    std::mem::discriminant(&html_err),
                    "html_escape changed error kind for {tmpl}: plain={plain_err:?} html={html_err:?}"
                );
                assert!(
                    !html_err.to_string().contains("__vael_auto_escape"),
                    "error leaked internal helper for {tmpl}: {html_err}"
                );
            }
            (plain, html) => {
                panic!("html_escape changed outcome for {tmpl}: plain={plain:?} html={html:?}")
            }
        }
    }
}

#[test]
fn html_escape_context_aware_attributes_and_urls() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 1. HTML text & quoted normal attributes
    assert_eq!(
        render(
            "<p>{{.x}}</p><div title=\"{{.x}}\"></div>",
            &json!({ "x": "<img src=x onerror=\"alert('1')\"> & more" }),
            &opts
        )
        .unwrap(),
        "<p>&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt; &amp; more</p><div title=\"&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt; &amp; more\"></div>"
    );

    // 2. Unquoted attributes: whitespace, =, /, ` are escaped so attribute breakout is impossible
    let unquoted = render(
        "<div class={{.x}}></div>",
        &json!({ "x": "x onmouseover=alert(1) data-a=`b`/c" }),
        &opts,
    )
    .unwrap();
    assert_eq!(
        unquoted,
        "<div class=x&#32;onmouseover&#61;alert(1)&#32;data-a&#61;&#96;b&#96;&#47;c></div>"
    );
    assert!(!unquoted.contains(" onmouseover="));

    // 3. URL attributes: dangerous schemes rejected, safe schemes & relative URLs allowed
    for dangerous in [
        "javascript:alert(1)",
        "  \t\nJaVaScRiPt:alert(1)",
        "\x01javascript:alert(1)",
        "java\tscript:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
    ] {
        let out = render(
            "<a href=\"{{.u}}\">link</a><form action={{.u}}></form>",
            &json!({ "u": dangerous }),
            &opts,
        )
        .unwrap();
        assert_eq!(
            out,
            "<a href=\"#ZgotmplZ\">link</a><form action=#ZgotmplZ></form>"
        );
    }

    for (safe_in, expected_href) in [
        (
            "https://example.com/a?x=1&y=2",
            "https://example.com/a?x=1&amp;y=2",
        ),
        ("HTTP://example.com", "HTTP://example.com"),
        ("mailto:ada@example.com", "mailto:ada@example.com"),
        ("tel:+15551234", "tel:+15551234"),
        ("/tasks/123?tab=open#top", "/tasks/123?tab=open#top"),
        ("./sub:path", "./sub:path"),
        ("?q=1&b=2", "?q=1&amp;b=2"),
        ("#section", "#section"),
    ] {
        let out = render("<a href=\"{{.u}}\">ok</a>", &json!({ "u": safe_in }), &opts).unwrap();
        assert_eq!(out, format!("<a href=\"{expected_href}\">ok</a>"));
    }

    // URL attribute continuation (after static prefix) still escapes attribute chars without breaking relative paths
    assert_eq!(
        render(
            "<form action=\"/tasks/{{.id}}/toggle\"></form>",
            &json!({ "id": "abc\" onclick=\"evil()" }),
            &opts
        )
        .unwrap(),
        "<form action=\"/tasks/abc&quot; onclick=&quot;evil()/toggle\"></form>"
    );

    // Conditional branches inside URL attribute both validate start-of-URL
    assert_eq!(
        render(
            "<a href=\"{{if .ok}}{{.a}}{{else}}{{.b}}{{end}}\">x</a>",
            &json!({ "ok": false, "a": "/ok", "b": "javascript:alert(1)" }),
            &opts
        )
        .unwrap(),
        "<a href=\"#ZgotmplZ\">x</a>"
    );
}

#[test]
fn html_escape_context_aware_scripts_styles_and_comments() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 4. Event-handler attributes (on*): JS-escape first, HTML-attribute-escape second
    let on_quoted = render(
        "<button onclick=\"f('{{.x}}')\">go</button>",
        &json!({ "x": "'); alert(1); //\"&#39;" }),
        &opts,
    )
    .unwrap();
    assert_eq!(
        on_quoted,
        "<button onclick=\"f('\\u0027); alert(1); \\u002f\\u002f\\u0022\\u0026#39;')\">go</button>"
    );

    let on_raw = render(
        "<button onclick=\"f({{.x}})\">go</button>",
        &json!({ "x": "1); alert(1)//" }),
        &opts,
    )
    .unwrap();
    assert_eq!(
        on_raw,
        "<button onclick=\"f(&quot;1); alert(1)\\u002f\\u002f&quot;)\">go</button>"
    );

    // 5. <script> context: quoted strings, template literals, and raw JS expressions
    let script_str = render(
        "<script>var a = \"{{.s}}\"; var b = `{{.s}}`; var c = {{.n}}; var d = {{.rawStr}};</script>",
        &json!({
            "s": "\";alert(1);//</script>${evil}\u{2028}\u{2029}",
            "n": 42,
            "rawStr": "</script><script>alert(1)</script>"
        }),
        &opts,
    )
    .unwrap();
    assert!(!script_str.contains("</script><script>"));
    assert_eq!(
        script_str,
        "<script>var a = \"\\u0022;alert(1);\\u002f\\u002f\\u003c\\u002fscript\\u003e${evil}\\u2028\\u2029\"; var b = `\\u0022;alert(1);\\u002f\\u002f\\u003c\\u002fscript\\u003e\\u0024{evil}\\u2028\\u2029`; var c = 42; var d = \"\\u003c\\u002fscript\\u003e\\u003cscript\\u003ealert(1)\\u003c\\u002fscript\\u003e\";</script>"
    );

    // 6. <style> and style="..." context
    assert_eq!(
        render(
            "<div style=\"color: {{.safe}}; width: {{.bad}}\"></div><style>p { color: {{.styleTag}}; }</style>",
            &json!({
                "safe": "#1a2b3c",
                "bad": "red; background: url(javascript:alert(1))",
                "styleTag": "</style><script>alert(1)</script>"
            }),
            &opts
        )
        .unwrap(),
        "<div style=\"color: #1a2b3c; width: ZgotmplZ\"></div><style>p { color: ZgotmplZ; }</style>"
    );

    for css_attack in [
        "expression(alert(1))",
        "url(https://evil.com)",
        "@import 'x'",
        "behavior",
        "-moz-binding",
        "10px /* comment */",
        "10px; } body { color: red",
    ] {
        assert_eq!(
            render(
                "<div style=\"color: {{.c}}\"></div>",
                &json!({ "c": css_attack }),
                &opts
            )
            .unwrap(),
            "<div style=\"color: ZgotmplZ\"></div>",
            "expected CSS attack {css_attack:?} to be replaced with ZgotmplZ"
        );
    }

    // 7. HTML comments: -- and > are sanitized so comment breakout is impossible
    let comment_out = render(
        "<!-- {{.c}} -->",
        &json!({ "c": "x --> <script>alert(1)</script> a--b" }),
        &opts,
    )
    .unwrap();
    assert_eq!(
        comment_out,
        "<!-- x &#45;&#45;&gt; &lt;script&gt;alert(1)&lt;/script&gt; a&#45;&#45;b -->"
    );
    assert!(!comment_out[4..comment_out.len() - 4].contains("--"));
    assert!(!comment_out[4..comment_out.len() - 4].contains('>'));
}

#[test]
fn html_escape_hardening_blocks_branch_divergence_non_text_calls_srcdoc_and_dynamic_names() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 1. Branch convergence rejects divergent contexts at compile time.
    for divergent in [
        "{{if .c}}<!--{{end}}<script>var a=\"{{.x}}\";</script>",
        "<script>{{if .q}}var s=\"{{end}}{{.x}}</script>",
        "{{if .c}}<div title=\"{{else}}<span class=\"{{end}}{{.x}}\">",
        "{{with .c}}<!--{{end}}<p>{{.x}}</p>",
    ] {
        assert!(
            matches!(
                render(divergent, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected compile Parse error for divergent template {divergent:?}"
        );
    }

    // 1b. Conservative URL-start join across `if` without `else`.
    assert_eq!(
        render(
            "<a href=\"{{if .p}}/v2/{{end}}{{.u}}\">x</a>",
            &json!({ "p": false, "u": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"#ZgotmplZ\">x</a>"
    );
    assert_eq!(
        render(
            "<a href=\"{{if .p}}/v2/{{end}}{{.u}}\">x</a>",
            &json!({ "p": true, "u": "https://example.com" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"/v2/https://example.com\">x</a>"
    );

    // 2. Reject {{template}} and {{block}} outside HtmlState::Text (standalone and TemplateSet).
    for bad_call in [
        "{{define \"u\"}}{{.u}}{{end}}<a href=\"{{template \"u\" .}}\">x</a>",
        "<a href=\"{{block \"u\" .}}{{.u}}{{end}}\">x</a>",
        "{{define \"u\"}}{{.u}}{{end}}<script>var x = {{template \"u\" .}};</script>",
        "{{define \"u\"}}{{.u}}{{end}}<!-- {{template \"u\" .}} -->",
    ] {
        assert!(
            matches!(
                render(bad_call, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected Parse error for non-text template/block call {bad_call:?}"
        );
    }
    let mut set = TemplateSet::new(opts.clone());
    set.add("u", "{{.u}}").unwrap();
    assert!(matches!(
        set.add("bad_href", "<a href=\"{{template \"u\" .}}\">x</a>"),
        Err(TemplateError::Parse(_))
    ));

    // 4. Reject dynamic actions inside <iframe srcdoc="...">.
    assert!(matches!(
        render(
            "<iframe srcdoc=\"{{.x}}\"></iframe>",
            &json!({ "x": "<script>" }),
            &opts
        ),
        Err(TemplateError::Parse(_))
    ));
    assert!(matches!(
        render(
            "<iframe SRCDOC={{.x}}></iframe>",
            &json!({ "x": "ok" }),
            &opts
        ),
        Err(TemplateError::Parse(_))
    ));

    // 8. Reject dynamic tag names and attribute names.
    for bad_tag_or_attr in [
        "<{{.tag}}>",
        "<div {{.attr}}=\"{{.val}}\">",
        "<div data-{{.k}}=\"1\">",
    ] {
        assert!(
            matches!(
                render(bad_tag_or_attr, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected Parse error for dynamic tag/attr {bad_tag_or_attr:?}"
        );
    }
}

#[test]
fn html_escape_hardening_blocks_marker_bypass_url_split_comment_and_js_lexer_pocs() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 3. escapeHtml marker only bypasses HTML text & quoted normal attributes, never URL/unquoted/on*.
    assert_eq!(
        render(
            "<a href=\"{{ .u | escapeHtml }}\">x</a>",
            &json!({ "u": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"#ZgotmplZ\">x</a>"
    );
    assert_eq!(
        render(
            "<div class={{ .c | escapeHtml }}></div>",
            &json!({ "c": "x onmouseover=alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<div class=x&#32;onmouseover&#61;alert(1)></div>"
    );
    assert_eq!(
        render(
            "<button onclick=\"f('{{ .x | escapeHtml }}')\">go</button>",
            &json!({ "x": "'); alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"f('\\u0026#39;); alert(1)\\u002f\\u002f')\">go</button>"
    );
}

#[test]
fn html_escape_hardening_blocks_comments_script_double_escape_and_url_split() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 5. HTML5 abrupt/bang comment closing, leading-hyphen neutralization, and <script> double-escape.
    assert_eq!(
        render(
            "<!--><script>var x=\"{{.u}}\";</script>",
            &json!({ "u": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<!--><script>var x=\"\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<!-- a --!><script>var x=\"{{.u}}\";</script>",
            &json!({ "u": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<!-- a --!><script>var x=\"\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render("<!-- {{.c}} -->", &json!({ "c": "-!-" }), &opts,).unwrap(),
        "<!-- &#45;&#33;&#45; -->"
    );
    assert_eq!(
        render(
            "<!-- hello {{.name}} -->",
            &json!({ "name": "world --> <script>" }),
            &opts,
        )
        .unwrap(),
        "<!-- hello world &#45;&#45;&gt; &lt;script&gt; -->"
    );
    assert_eq!(
        render(
            "<script><!-- <script> </script> var x={{.u}}; --></script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script><!-- <script> </script> var x=\"alert(1)\"; --></script>"
    );

    // 6. URL multi-action & leading HTML entity handling.
    assert_eq!(
        render(
            "<a href=\"{{.a}}{{.b}}\">x</a>",
            &json!({ "a": "", "b": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"#ZgotmplZ\">x</a>"
    );
    assert_eq!(
        render(
            "<a href=\"{{.a}}{{.b}}\">x</a>",
            &json!({ "a": "javascript", "b": ":alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"javascript#ZgotmplZ\">x</a>"
    );
    assert_eq!(
        render(
            "<a href=\"&Tab;{{.u}}\">x</a>",
            &json!({ "u": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<a href=\"&Tab;#ZgotmplZ\">x</a>"
    );
}

#[test]
fn html_escape_hardening_blocks_js_lexer_and_bogus_comment_pocs() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 7. JS lexer & on* HTML entity decoding + bogus comment `</1`.
    assert_eq!(
        render(
            "<button onclick=\"/* ' */ {{.x}}\">go</button>",
            &json!({ "x": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"/* ' */ &quot;alert(1)&quot;\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"f(&#96;{{.x}}&#96;)\">go</button>",
            &json!({ "x": "${alert(1)}" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"f(&#96;\\u0024{alert(1)}&#96;)\">go</button>"
    );
    assert_eq!(
        render(
            "<script>var r=/\\\"/; var x={{.u}};</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var r=/\\\"/; var x=\"alert(1)\";</script>"
    );
    assert_eq!(
        render(
            "<script>var s=`a ${ {{.u}} }`;</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var s=`a ${ \"alert(1)\" }`;</script>"
    );
    assert_eq!(
        render(
            "<script>var s=`a ${\"`\"}`; var x={{.u}};</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var s=`a ${\"`\"}`; var x=\"alert(1)\";</script>"
    );
    assert_eq!(
        render(
            "<script>var x=1; <!-- \"\n var y={{.u}};</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var x=1; <!-- \"\n var y=\"alert(1)\";</script>"
    );
    assert_eq!(
        render(
            "</1 title=\"><script>var x={{.u}};</script>\">",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "</1 title=\"><script>var x=\"alert(1)\";</script>\">"
    );
}

#[test]
fn html_escape_hardening_rejects_non_text_eof_and_comment_substate_pocs() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 1. Non-Text EOF state rejected in CompiledTemplate and TemplateSet (main, define, block).
    for unclosed in [
        "<script>var x = {{.x}};",
        "<script>",
        "<!-- {{.x}}",
        "<div title=\"{{.x}}",
        "<style>p { color: {{.x}};",
        "<textarea>{{.x}}",
        "<title>{{.x}}",
    ] {
        assert!(
            matches!(
                render(unclosed, &json!({ "x": "1" }), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected EOF Parse error for {unclosed:?}"
        );
    }

    let mut open_set = TemplateSet::new(opts.clone());
    assert!(matches!(
        open_set.add("open", "<script>"),
        Err(TemplateError::Parse(_))
    ));
    assert!(matches!(
        open_set.add("defs", "{{define \"open\"}}<script>{{end}}"),
        Err(TemplateError::Parse(_))
    ));
    assert!(matches!(
        open_set.add("blk", "{{block \"open\" .}}<script>{{end}}"),
        Err(TemplateError::Parse(_))
    ));
    assert!(matches!(
        open_set.add("open_comment", "<!-- {{.x}}"),
        Err(TemplateError::Parse(_))
    ));
    assert!(matches!(
        open_set.add("open_attr", "<div title=\"{{.x}}"),
        Err(TemplateError::Parse(_))
    ));

    // 2. HTML5 comment substates, branch equality, and close-token lookahead.
    for bad_comment in [
        "<!--{{if .x}}foo{{end}}><script>var a=\"{{.u}}\";</script>",
        "<!--{{.c}}><script>var x=\"{{.u}}\";</script>",
        "<!---{{.c}}><script>var x=\"{{.u}}\";</script>",
        "<!-- a --{{.c}}><script>var x=\"{{.u}}\";</script>",
        "<!-- a --!{{.c}}><script>var x=\"{{.u}}\";</script>",
        "<!-- a -{{.c}}->",
        "<!-- -{{.c}}><script>var x=\"{{.u}}\";</script>",
        "<!-- {{if .x}}--{{end}}> -->",
        "<!-- {{if .x}}foo{{end}}>",
        "<!-- {{.a}}{{if .x}}foo{{end}}>",
        "<!-- {{.c}}->",
        "<!-- {{.c}}!>",
        "<!-- {{- .c}} -->",
        "<!-- {{.c -}} > -->",
        "<!{{if .c}}--{{end}}foo><script>var a=\"{{.u}}\";</script>",
    ] {
        assert!(
            matches!(
                render(bad_comment, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected compile Parse error for comment PoC {bad_comment:?}"
        );
    }
    assert_eq!(
        render(
            "<!-- {{if .x}}hello{{else}}world{{end}} -->",
            &json!({ "x": true }),
            &opts
        )
        .unwrap(),
        "<!-- hello -->"
    );
}

#[test]
fn html_escape_hardening_handles_utf8_entities_and_js_line_continuations() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 3. UTF-8 panic DoS & WHATWG numeric/named HTML entity decoding in `on*`.
    assert_eq!(
        render(
            "<button onclick=\"&aaaaaaaaaaaaaa\u{00e9}; f({{.x}})\">go</button>",
            &json!({ "x": "1);alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"&aaaaaaaaaaaaaa\u{00e9}; f(&quot;1);alert(1)\\u002f\\u002f&quot;)\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"f(&#96+{{.x}}+&#96)\">go</button>",
            &json!({ "x": "${alert(1)}" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"f(&#96+\\u0024{alert(1)}+&#96)\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"f(&#00000000000000000096;{{.x}}&#00000000000000000096;)\">go</button>",
            &json!({ "x": "${alert(1)}" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"f(&#00000000000000000096;\\u0024{alert(1)}&#00000000000000000096;)\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"&#47* ' */ {{.x}}\">go</button>",
            &json!({ "x": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"&#47* ' */ &quot;alert(1)&quot;\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"&#0000000000000000047;* &#39; */ {{.x}}\">go</button>",
            &json!({ "x": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"&#0000000000000000047;* &#39; */ &quot;alert(1)&quot;\">go</button>"
    );
    assert_eq!(
        render(
            "<button onclick=\"&#x99999999999999999999; f(&quot+{{.x}}+&quot)\">go</button>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<button onclick=\"&#x99999999999999999999; f(&quot+\\u0022;alert(1)\\u002f\\u002f+&quot)\">go</button>"
    );

    // 4. JS LineContinuation (`\<newline>`, `\<CR><LF>`, `\u2028`, `\u2029`) inside quoted strings.
    assert_eq!(
        render(
            "<script>var s = \"\\\n{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var s = \"\\\n\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<script>var s = \"\\\r\n{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var s = \"\\\r\n\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<script>var s = \"\\\r{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var s = \"\\\r\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<script>var s = \"\\\u{2028}{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var s = \"\\\u{2028}\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
}

#[test]
fn html_escape_hardening_fail_closes_js_regex_division_ambiguity() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 5. Fail-closed JS regex-vs-division disambiguation & `++`/`--` & `.return` & trailing `\`/`$`.
    for bad_js in [
        "<script>{{if .c}}var x=1{{else}}var x=1+{{end}} /a\"/; var y={{.u}}+\"\";</script>",
        "<script>{{if .c}}var x=1 {{else}}var x=1+{{end}} /a\"/; var y={{.u}}+\"\";</script>",
        "<script>var x={} /a\"/; var y={{.u}}+\"\";</script>",
        "<script>if (1) /\"/; var y={{.u}};</script>",
        "<script>if (1) /* c */ /\"/; var y={{.u}};</script>",
        "<button onclick=\"if (1) /'/; {{.u}}\">go</button>",
        "<script>var s = \"\\{{.x}}\";</script>",
        "<script>var s = `${{.x}}`;</script>",
        "<script>var r = /{{.x}}/; var y = {{.u}};</script>",
        "<script>{{if .c}}\n{{else}}var x=1;{{end}}--> \"\nvar y={{.u}};</script>",
        "<script>var s=\"</scr{{.x}}ipt>\";</script>",
        "<style>p{}</style{{.x}}>",
    ] {
        assert!(
            matches!(
                render(bad_js, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected compile Parse error for JS/tag PoC {bad_js:?}"
        );
    }
    assert_eq!(
        render(
            "<script>var y = x++ / b + \"{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var y = x++ / b + \"\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<script>var y = x-- / b + \"{{.x}}\";</script>",
            &json!({ "x": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var y = x-- / b + \"\\u0022;alert(1)\\u002f\\u002f\";</script>"
    );
    assert_eq!(
        render(
            "<script>var x = obj.return /a\"/; var y={{.u}}+\"\";</script>",
            &json!({ "u": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = obj.return /a\"/; var y=\\u0022;alert(1)\\u002f\\u002f+\"\";</script>"
    );
    assert_eq!(
        render(
            "<script>var x = obj.return / a; var y={{.u}};</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = obj.return / a; var y=\"alert(1)\";</script>"
    );
    assert_eq!(
        render(
            "<script>var x = 1 + /* {{.x}} */ /\"/; var y={{.u}};</script>",
            &json!({ "x": "ok", "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = 1 + /* \"ok\" */ /\"/; var y=\"alert(1)\";</script>"
    );
}

#[test]
fn html_escape_hardening_rejects_unsafe_url_schemes_and_isolates_rcdata() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 6. URL attribute static unsafe schemes & static `:` after dynamic prefix.
    for bad_url_tpl in [
        "<a href=\"javascript:{{.u}}\">x</a>",
        "<a href=\"vbscript:{{.u}}\">x</a>",
        "<a href=\"data:{{.u}}\">x</a>",
        "<a href=\"java&#115;cript:{{.u}}\">x</a>",
        "<a href=\"java&#x09;script:{{.u}}\">x</a>",
        "<a href=\"javascript&#58;{{.u}}\">x</a>",
        "<a href=\"javascript&colon;{{.u}}\">x</a>",
        "<a href=javascript:{{.u}}>x</a>",
        "<a href=\"{{.a}}:{{.b}}\">x</a>",
        "<a href=\"{{.a}}:alert(1)\">x</a>",
        "<a href=\"{{.a}}&#58;alert(1)\">x</a>",
        "<a href=\"{{.a}}&colon;alert(1)\">x</a>",
        "<a href=\"{{.a}}&#58{{.b}}\">x</a>",
        "<a href=\"{{if .c}}java{{end}}:alert(1)\">x</a>",
        "<a href=\"{{if .c}}java{{end}}http:{{.u}}\">x</a>",
    ] {
        assert!(
            matches!(
                render(bad_url_tpl, &json!({}), &opts),
                Err(TemplateError::Parse(_))
            ),
            "expected compile Parse error for URL PoC {bad_url_tpl:?}"
        );
    }
    assert_eq!(
        render(
            "<img src=\"data:image/png;base64,iVBOR\"><a href=\"https&#58;//example.com/{{.p}}:8080\">ok</a>",
            &json!({ "p": "v1" }),
            &opts,
        )
        .unwrap(),
        "<img src=\"data:image/png;base64,iVBOR\"><a href=\"https&#58;//example.com/v1:8080\">ok</a>"
    );

    // 7. RCDATA isolation (`<textarea>`, `<title>`).
    assert_eq!(
        render(
            "<textarea><script></textarea><a href={{.u}}>",
            &json!({ "u": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<textarea><script></textarea><a href=#ZgotmplZ>"
    );
    assert_eq!(
        render(
            "<title><!--</title><a href=\"{{.u}}\">x</a>",
            &json!({ "u": "javascript:alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<title><!--</title><a href=\"#ZgotmplZ\">x</a>"
    );
}

#[test]
fn html_escape_hardening_handles_consecutive_comments_decimal_dot_noscript_asi_and_unicode_ident() {
    let opts = TemplateOptions {
        html_escape: true,
        ..TemplateOptions::default()
    };

    // 1. Consecutive `Token::Comment` trim scanning and coalescing.
    assert_eq!(
        render(
            "<!-- {{/* a */}}{{- /* b */}}><script>var a=\"{{.u}}\";</script>-->",
            &json!({ "u": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<!--><script>var a=\"\\u0022;alert(1)\\u002f\\u002f\";</script>-->"
    );
    assert_eq!(
        render(
            "<script>/ {{/* a */}}{{- /* b */}}* / ' */ var y = {{.u}}; /* ' */</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>/* / ' */ var y = \"alert(1)\"; /* ' */</script>"
    );

    // 2. Trailing decimal point (`1.\n`) does not suppress keyword recognition, while `1..toString()` still marks member access.
    assert_eq!(
        render(
            "<script>var x = 1.\ntypeof /\"/; var y = {{.u}}; // \"\n</script>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = 1.\ntypeof /\"/; var y = \"alert(1)\"; // \"\n</script>"
    );
    assert_eq!(
        render(
            "<script>var x = 1..typeof /\"/; var y = {{.u}}+\"\";</script>",
            &json!({ "u": "\";alert(1)//" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = 1..typeof /\"/; var y = \\u0022;alert(1)\\u002f\\u002f+\"\";</script>"
    );

    // 3. `<noscript>` as `HtmlState::RawText`.
    assert_eq!(
        render(
            "<noscript><div title=\"</noscript><script>var x={{.u}};</script>\"></noscript>",
            &json!({ "u": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<noscript><div title=\"</noscript><script>var x=\"alert(1)\";</script>\"></noscript>"
    );

    // 4. Labeled `break` / `continue` ASI before `/`.
    assert_eq!(
        render(
            "<script>l: { break l\n/a'b/; } var y = {{.x}};</script>",
            &json!({ "x": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>l: { break l\n/a'b/; } var y = \"alert(1)\";</script>"
    );
    assert_eq!(
        render(
            "<script>l: for (;;) { continue l\n/a'b/; } var y = {{.x}};</script>",
            &json!({ "x": "alert(1)" }),
            &opts,
        )
        .unwrap(),
        "<script>l: for (;;) { continue l\n/a'b/; } var y = \"alert(1)\";</script>"
    );

    // 5. Non-ASCII JS identifier chars (`éreturn` is not keyword `return`).
    assert_eq!(
        render(
            "<script>var x = \u{00e9}return / 2 + \"{{.u}}\";</script>",
            &json!({ "u": "\";alert(1);//" }),
            &opts,
        )
        .unwrap(),
        "<script>var x = \u{00e9}return / 2 + \"\\u0022;alert(1);\\u002f\\u002f\";</script>"
    );
}
