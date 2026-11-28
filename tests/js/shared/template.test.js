import test from "vael:test";
import assert from "vael:assert";
import templateDefault, * as templateNs from "vael:template";
import { compile, createTemplateSet, escapeHtml, render } from "vael:template";

function expectTemplateError(fn, expectedCode) {
    let caught = null;
    try {
        fn();
    } catch (error) {
        caught = error;
    }
    assert.ok(caught instanceof Error, `expected Error (${expectedCode}), got ${caught}`);
    assert.strictEquals(
        caught.code,
        expectedCode,
        `expected code ${expectedCode}, got ${caught.code} (${caught.message})`,
    );
    assert.strictEquals(typeof caught.message, "string");
    assert.ok(caught.message.length > 0, "error message must not be empty");
    return caught;
}

test("vael:template registry, named exports, and default export share frozen identity", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:template"), "vael:template missing from Vael.getBuiltInModulesList()");
    assert.ok(!list.includes("template"), "bare template must not be listed");
    assert.strictEquals(Vael.getBuiltInModule("template"), undefined);

    const registry = Vael.getBuiltInModule("vael:template");
    assert.ok(registry !== undefined && typeof registry === "object" && registry !== null);
    assert.strictEquals(Vael.getBuiltInModule("vael:template"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(templateDefault, registry);
    assert.ok(templateNs !== registry, "module namespace must be distinct from registry");

    const dyn = await import("vael:template");
    assert.strictEquals(dyn, templateNs);

    const expected = ["compile", "createTemplateSet", "escapeHtml", "render"];
    assert.equals(Object.keys(registry).sort(), expected);
    assert.equals(Object.keys(templateNs).sort(), ["compile", "createTemplateSet", "default", "escapeHtml", "render"]);

    assert.strictEquals(render, registry.render);
    assert.strictEquals(compile, registry.compile);
    assert.strictEquals(createTemplateSet, registry.createTemplateSet);
    assert.strictEquals(escapeHtml, registry.escapeHtml);
    assert.strictEquals(templateDefault.render, render);
    assert.strictEquals(templateDefault.compile, compile);
    assert.strictEquals(templateDefault.createTemplateSet, createTemplateSet);
    assert.strictEquals(templateDefault.escapeHtml, escapeHtml);
});

test("render supports Go template syntax (dot paths, if, range, with, pipes)", () => {
    assert.strictEquals(render("Hello, {{.name}}!", { name: "Vael" }), "Hello, Vael!");
    assert.strictEquals(render("Static"), "Static");

    const tpl = "{{if .ok}}YES{{else}}NO{{end}}:[{{range .items}}{{.}},{{end}}]:{{with .user}}{{.name}}{{end}}";
    assert.strictEquals(
        render(tpl, { ok: true, items: [1, 2, 3], user: { name: "Ada" } }),
        "YES:[1,2,3,]:Ada",
    );
    assert.strictEquals(
        render(tpl, { ok: false, items: [], user: null }),
        "NO:[]:",
    );

    // Omitted / undefined data defaults to empty object context.
    assert.strictEquals(render("plain {{if .ok}}x{{else}}ok{{end}}", { ok: false }), "plain ok");
    assert.strictEquals(render("plain", undefined), "plain");
});

test("render supports Sprig functions (upper, trim, default, join, add, dict, list)", () => {
    assert.strictEquals(
        render("{{.s | trim | upper}}", { s: "  hello  " }),
        "HELLO",
    );
    assert.strictEquals(
        render('{{.empty | default "fallback"}}', { empty: "" }),
        "fallback",
    );
    assert.strictEquals(
        render('{{list "a" "b" "c" | join "-"}}', {}),
        "a-b-c",
    );
    assert.strictEquals(
        render("{{add 2 3 4}}", {}),
        "9",
    );
    assert.strictEquals(
        render('{{$d := dict "k" "v"}}{{$d.k}}', {}),
        "v",
    );
});

test("sprig: false disables Sprig helpers while keeping Go builtins", () => {
    assert.strictEquals(
        render("{{len .items}}", { items: [10, 20, 30] }, { sprig: false }),
        "3",
    );
    const err = expectTemplateError(
        () => render("{{.s | upper}}", { s: "abc" }, { sprig: false }),
        "RENDER",
    );
    assert.ok(err.message.includes("upper"));
});

test("hermetic: true excludes clock and environment helpers", () => {
    assert.strictEquals(
        render("{{.s | upper}}", { s: "ok" }, { hermetic: true }),
        "OK",
    );
    for (const fn of ["now", "ago"]) {
        let caught = null;
        try {
            render(`{{${fn}}}`, {}, { hermetic: true });
        } catch (err) {
            caught = err;
        }
        assert.ok(caught instanceof Error, `expected error for hermetic ${fn}`);
        assert.ok(
            caught.code === "PARSE" || caught.code === "RENDER",
            `unexpected error code for hermetic ${fn}: ${caught.code}`,
        );
    }
});

test("compile produces a frozen branded Template reusable across renders", () => {
    const tmpl = compile("Hi {{.user | upper}} (#{{.id}})");
    assert.strictEquals(Object.isFrozen(tmpl), true);
    assert.strictEquals(tmpl.render({ user: "ada", id: 1 }), "Hi ADA (#1)");
    assert.strictEquals(tmpl.render({ user: "grace", id: 2 }), "Hi GRACE (#2)");
    assert.strictEquals(compile("fixed").render(), "fixed");

    // Branded receiver check: calling render on a forged object must fail.
    assert.throws(() => tmpl.render.call({}, { user: "x", id: 0 }), TypeError);
    assert.throws(() => {
        tmpl.render = () => "hijacked";
    }, TypeError);

    // Object.prototype.render setter tampering cannot hijack compiled template methods.
    Object.defineProperty(Object.prototype, "render", {
        set() {
            throw new Error("Object.prototype.render setter hijacked!");
        },
        configurable: true,
    });
    try {
        const tmpl2 = compile("Safe {{.x}}");
        assert.strictEquals(tmpl2.render({ x: 42 }), "Safe 42");
    } finally {
        delete Object.prototype.render;
    }
});

test("createTemplateSet composes layouts and partials with chainable add and sorted names", () => {
    const set = createTemplateSet();
    assert.strictEquals(Object.isFrozen(set), true);

    const chained = set
        .add("partial", "<span>{{.tag}}</span>")
        .add("layout", "<html><title>{{block \"title\" .}}Site{{end}}</title><body>{{template \"content\" .}}</body></html>")
        .add(
            "home",
            "{{define \"title\"}}Home{{end}}{{define \"content\"}}<h1>Hi {{.name}}</h1>{{template \"partial\" .}}{{end}}{{template \"layout\" .}}",
        );

    assert.strictEquals(chained, set, "add() must return the TemplateSet for chaining");
    assert.equals(set.names(), ["home", "layout", "partial"]);
    assert.strictEquals(
        set.render("home", { name: "Vael", tag: "v1" }),
        "<html><title>Home</title><body><h1>Hi Vael</h1><span>v1</span></body></html>",
    );

    assert.throws(() => set.add.call({}, "a", "b"), TypeError);
    assert.throws(() => set.render.call({}, "home", {}), TypeError);
    assert.throws(() => set.names.call({}), TypeError);
});

test("htmlEscape: true, safeHtml, and escapeHtml protect against XSS", () => {
    const escaped = escapeHtml(`<a href="x" title='y'>a & b</a>`);
    assert.strictEquals(escaped, "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;a &amp; b&lt;/a&gt;");

    const out = render(
        "<p>{{.untrusted}}</p>{{.trusted | safeHtml}}",
        {
            untrusted: '<script>alert("xss")</script>',
            trusted: "<em>ok</em>",
        },
        { htmlEscape: true },
    );
    assert.strictEquals(out, "<p>&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;</p><em>ok</em>");

    // Default htmlEscape is false.
    assert.strictEquals(render("{{.raw}}", { raw: "<b>raw</b>" }), "<b>raw</b>");
});

test("resource limits enforce LIMIT_EXCEEDED", () => {
    expectTemplateError(
        () => render("1234567890", {}, { maxSourceLength: 5 }),
        "LIMIT_EXCEEDED",
    );
    expectTemplateError(
        () => render("{{if true}}{{if true}}x{{end}}{{end}}", {}, { maxNestingDepth: 1 }),
        "LIMIT_EXCEEDED",
    );
    expectTemplateError(
        () => render("{{.s}}", { s: "hello world" }, { maxOutputLength: 5 }),
        "LIMIT_EXCEEDED",
    );
    expectTemplateError(
        () => render("{{range .items}}x{{end}}", { items: [1, 2, 3, 4] }, { maxIterations: 2 }),
        "LIMIT_EXCEEDED",
    );
    expectTemplateError(
        () => render("{{.a | upper | trim}}", { a: "hi" }, { maxFunctionCalls: 1 }),
        "LIMIT_EXCEEDED",
    );
});

test("structured errors carry documented .code values for invalid inputs and hostile values", () => {
    // PARSE
    expectTemplateError(() => render("{{if .x}}", {}), "PARSE");
    expectTemplateError(() => compile("{{range .x}}"), "PARSE");

    // RENDER
    expectTemplateError(() => render('{{fail "boom"}}', {}), "RENDER");

    // TEMPLATE_NOT_FOUND
    const set = createTemplateSet().add("a", "hello");
    expectTemplateError(() => set.render("missing", {}), "TEMPLATE_NOT_FOUND");
    expectTemplateError(() => render('{{template "missing" .}}', {}), "TEMPLATE_NOT_FOUND");

    // INVALID_NAME
    expectTemplateError(() => set.add("", "hi"), "INVALID_NAME");
    expectTemplateError(() => set.add("   ", "hi"), "INVALID_NAME");
    expectTemplateError(() => set.add("bad\nname", "hi"), "INVALID_NAME");
    expectTemplateError(() => set.add(123, "hi"), "INVALID_NAME");
    expectTemplateError(() => set.render(123, {}), "INVALID_NAME");

    // INVALID_DATA: non-string source / input
    expectTemplateError(() => render(123, {}), "INVALID_DATA");
    expectTemplateError(() => render(null, {}), "INVALID_DATA");
    expectTemplateError(() => compile(42), "INVALID_DATA");
    expectTemplateError(() => escapeHtml(99), "INVALID_DATA");
    expectTemplateError(() => set.add("ok", 123), "INVALID_DATA");

    // INVALID_DATA: invalid options
    expectTemplateError(() => render("hi", {}, null), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, []), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, { unknownOpt: true }), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, { sprig: "yes" }), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, { maxOutputLength: -1 }), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, { maxOutputLength: 1.5 }), "INVALID_DATA");
    expectTemplateError(() => render("hi", {}, { maxOutputLength: NaN }), "INVALID_DATA");
    expectTemplateError(() => createTemplateSet(new Proxy({}, {})), "INVALID_DATA");
    expectTemplateError(() => createTemplateSet({ get sprig() { return true; } }), "INVALID_DATA");

    // INVALID_DATA: non-object data & hostile values
    expectTemplateError(() => render("hi", null), "INVALID_DATA");
    expectTemplateError(() => render("hi", "str"), "INVALID_DATA");
    expectTemplateError(() => render("hi", 42), "INVALID_DATA");
    expectTemplateError(() => render("hi", [1, 2]), "INVALID_DATA");
    expectTemplateError(() => render("hi", new Map()), "INVALID_DATA");
    expectTemplateError(() => render("hi", new Proxy({}, {})), "INVALID_DATA");
    expectTemplateError(() => render("hi", { n: NaN }), "INVALID_DATA");
    expectTemplateError(() => render("hi", { n: Infinity }), "INVALID_DATA");
    expectTemplateError(() => render("hi", { b: 1n }), "INVALID_DATA");
    expectTemplateError(() => render("hi", { fn: () => {} }), "INVALID_DATA");
    expectTemplateError(() => render("hi", { sym: Symbol("x") }), "INVALID_DATA");
    expectTemplateError(
        () => render("hi", { get trap() { throw new Error("trap ran"); } }),
        "INVALID_DATA",
    );

    // Cyclic data
    const cyclic = {};
    cyclic.self = cyclic;
    expectTemplateError(() => render("hi", cyclic), "INVALID_DATA");

    const cyclicArr = [];
    cyclicArr.push(cyclicArr);
    expectTemplateError(() => render("hi", { items: cyclicArr }), "INVALID_DATA");

    // Unpaired UTF-16 surrogates
    expectTemplateError(() => render("bad\uD800", {}), "INVALID_DATA");
    expectTemplateError(() => escapeHtml("bad\uD800"), "INVALID_DATA");
    expectTemplateError(() => render("{{.s}}", { s: "bad\uD800" }), "INVALID_DATA");
});

test("regression: resists Array.prototype/Error.prototype tampering, sparse giant arrays, and exotic data objects", () => {
    // 1. Array.prototype["0"] setter on set.names()
    const set = createTemplateSet().add("beta", "b").add("alpha", "a");
    Object.defineProperty(Array.prototype, "0", {
        set() {
            throw new Error("Array.prototype[0] setter hijacked!");
        },
        configurable: true,
    });
    try {
        assert.equals(set.names(), ["alpha", "beta"]);
    } finally {
        delete Array.prototype[0];
    }

    // 2. Error.prototype.code and Object.prototype.code tampering
    Object.defineProperty(Error.prototype, "code", {
        value: "FORGED",
        writable: false,
        configurable: true,
    });
    Object.defineProperty(Object.prototype, "code", {
        set() {
            throw new Error("Object.prototype.code setter hijacked!");
        },
        configurable: true,
    });
    try {
        expectTemplateError(() => render("hi", {}, { unknownOpt: true }), "INVALID_DATA");
        expectTemplateError(() => render("hi", {}, { sprig: "no" }), "INVALID_DATA");
        expectTemplateError(() => render("hi", {}, new Proxy({}, {})), "INVALID_DATA");
        expectTemplateError(() => render("{{if}}", {}), "PARSE");
    } finally {
        delete Error.prototype.code;
        delete Object.prototype.code;
    }

    // 3. Giant array length (0x80000000) must fail cleanly without panic
    const giant = [];
    giant.length = 0x80000000;
    expectTemplateError(() => render("hi", { items: giant }), "INVALID_DATA");

    // 4. Multi-key sparse array bypassing single-array node limit
    const holey = new Array(30000);
    expectTemplateError(
        () => render("hi", { a: holey, b: holey, c: holey, d: holey }),
        "INVALID_DATA",
    );

    // 5. Proxy arrays (top-level and nested)
    expectTemplateError(() => render("hi", new Proxy([], {})), "INVALID_DATA");
    expectTemplateError(() => render("hi", { items: new Proxy([], {}) }), "INVALID_DATA");

    // 6. Disallowed exotic objects (WeakMap, WeakRef, Template, TemplateSet, ModuleNamespace)
    const compiled = compile("x");
    const tmplSet = createTemplateSet();
    expectTemplateError(() => render("hi", new WeakMap()), "INVALID_DATA");
    expectTemplateError(() => render("hi", { wm: new WeakMap() }), "INVALID_DATA");
    expectTemplateError(() => render("hi", compiled), "INVALID_DATA");
    expectTemplateError(() => render("hi", { t: compiled }), "INVALID_DATA");
    expectTemplateError(() => render("hi", tmplSet), "INVALID_DATA");
    expectTemplateError(() => render("hi", { s: tmplSet }), "INVALID_DATA");
    expectTemplateError(() => render("hi", templateNs), "INVALID_DATA");
    expectTemplateError(() => render("hi", { ns: templateNs }), "INVALID_DATA");
    if (typeof WeakRef === "function") {
        const wr = new WeakRef({});
        expectTemplateError(() => render("hi", wr), "INVALID_DATA");
        expectTemplateError(() => render("hi", { wr }), "INVALID_DATA");
    }
});

test("htmlEscape: true provides context-aware OWASP XSS protection and exempts control keywords", () => {
    const opts = { htmlEscape: true };

    // 1. HTML text and quoted normal attributes
    assert.strictEquals(
        render("<p>{{.x}}</p><div title=\"{{.x}}\"></div>", { x: '<img src=x onerror="alert(1)"> & more' }, opts),
        '<p>&lt;img src=x onerror=&quot;alert(1)&quot;&gt; &amp; more</p><div title="&lt;img src=x onerror=&quot;alert(1)&quot;&gt; &amp; more"></div>',
    );

    // 2. Unquoted attributes: whitespace, =, /, ` are escaped so attribute breakout is impossible
    const unquoted = render("<div class={{.x}}></div>", { x: "x onmouseover=alert(1) data-a=`b`/c" }, opts);
    assert.strictEquals(
        unquoted,
        "<div class=x&#32;onmouseover&#61;alert(1)&#32;data-a&#61;&#96;b&#96;&#47;c></div>",
    );
    assert.ok(!unquoted.includes(" onmouseover="));

    // 3. URL attributes: dangerous schemes rejected, safe schemes & relative URLs allowed
    for (const badUrl of [
        "javascript:alert(1)",
        "  \t\nJaVaScRiPt:alert(1)",
        "\x01javascript:alert(1)",
        "java\tscript:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
    ]) {
        assert.strictEquals(
            render('<a href="{{.u}}">link</a><form action={{.u}}></form>', { u: badUrl }, opts),
            '<a href="#ZgotmplZ">link</a><form action=#ZgotmplZ></form>',
        );
    }
    assert.strictEquals(
        render('<a href="{{.u}}">ok</a>', { u: "https://example.com/a?x=1&y=2" }, opts),
        '<a href="https://example.com/a?x=1&amp;y=2">ok</a>',
    );
    assert.strictEquals(
        render('<a href="{{.u}}">ok</a>', { u: "/tasks/123?tab=open#top" }, opts),
        '<a href="/tasks/123?tab=open#top">ok</a>',
    );
    assert.strictEquals(
        render('<a href="{{.u}}">ok</a>', { u: "mailto:ada@example.com" }, opts),
        '<a href="mailto:ada@example.com">ok</a>',
    );
    assert.strictEquals(
        render('<a href="{{.u}}">ok</a>', { u: "tel:+15551234" }, opts),
        '<a href="tel:+15551234">ok</a>',
    );

    // 4. Event-handler attributes (on*) + <script> context
    assert.strictEquals(
        render("<button onclick=\"f('{{.x}}')\">go</button>", { x: "'); alert(1); //\"" }, opts),
        "<button onclick=\"f('\\u0027); alert(1); \\u002f\\u002f\\u0022')\">go</button>",
    );
    const scriptOut = render(
        '<script>var a = "{{.s}}"; var b = `{{.s}}`; var n = {{.n}}; var r = {{.r}};</script>',
        {
            s: '";alert(1);//</script>${evil}\u2028\u2029',
            n: 42,
            r: "</script><script>alert(1)</script>",
        },
        opts,
    );
    assert.ok(!scriptOut.includes("</script><script>"));
    assert.strictEquals(
        scriptOut,
        '<script>var a = "\\u0022;alert(1);\\u002f\\u002f\\u003c\\u002fscript\\u003e${evil}\\u2028\\u2029"; var b = `\\u0022;alert(1);\\u002f\\u002f\\u003c\\u002fscript\\u003e\\u0024{evil}\\u2028\\u2029`; var n = 42; var r = "\\u003c\\u002fscript\\u003e\\u003cscript\\u003ealert(1)\\u003c\\u002fscript\\u003e";</script>',
    );

    // 5. <style> / style="..." + HTML comments (<!-- ... -->)
    assert.strictEquals(
        render(
            '<div style="color: {{.good}}; width: {{.bad}}"></div><style>p { color: {{.tag}}; }</style><!-- {{.comment}} -->',
            {
                good: "#1a2b3c",
                bad: "expression(alert(1))",
                tag: "</style><script>alert(1)</script>",
                comment: "x --> <script>alert(1)</script> a--b",
            },
            opts,
        ),
        '<div style="color: #1a2b3c; width: ZgotmplZ"></div><style>p { color: ZgotmplZ; }</style><!-- x &#45;&#45;&gt; &lt;script&gt;alert(1)&lt;/script&gt; a&#45;&#45;b -->',
    );

    // Variable declarations ($x := ..., $k, $v := ...) and non-assignment '=' in expressions
    assert.strictEquals(
        render(
            '{{$x := "<b>hi</b>"}}{{$x}}|{{range $k, $v := .list}}{{$k}}={{$v}};{{end}}|{{$y := ""}}{{$y | default "a=<b"}}',
            { list: ["<x>", "y"] },
            opts,
        ),
        "&lt;b&gt;hi&lt;/b&gt;|0=&lt;x&gt;;1=y;|a=&lt;b",
    );

    // Control keywords / reassignments (else if, else with, break, continue, $x = ...)
    // must match htmlEscape: false behavior without wrapping into __vael_auto_escape.
    for (const src of [
        "{{if .a}}A{{else if .b}}B{{end}}",
        "{{with .a}}A{{else with .b}}B{{end}}",
        "{{range .items}}{{break}}{{end}}",
        "{{range .items}}{{continue}}{{end}}",
        "{{$x := 1}}{{$x = 2}}{{$x}}",
    ]) {
        let plainOut = null;
        let plainErr = null;
        try {
            plainOut = render(src, { a: false, b: true, items: [1] }, { htmlEscape: false });
        } catch (e) {
            plainErr = e;
        }
        let htmlOut = null;
        let htmlErr = null;
        try {
            htmlOut = render(src, { a: false, b: true, items: [1] }, opts);
        } catch (e) {
            htmlErr = e;
        }
        if (plainErr !== null) {
            assert.ok(htmlErr instanceof Error);
            assert.strictEquals(htmlErr.code, plainErr.code);
            assert.ok(!htmlErr.message.includes("__vael_auto_escape"));
        } else {
            assert.strictEquals(htmlErr, null);
            assert.strictEquals(htmlOut, plainOut);
        }
    }
});

test("htmlEscape: true security hardening blocks branch divergence, marker bypass, URL splits, comments, srcdoc, and JS lexer PoCs", () => {
    const opts = { htmlEscape: true };

    // Compile-time rejections: branch divergence, non-text template/block calls, srcdoc, dynamic tag/attr names
    for (const bad of [
        '{{if .c}}<!--{{end}}<script>var a="{{.x}}";</script>',
        '<script>{{if .q}}var s="{{end}}{{.x}}</script>',
        '{{define "u"}}{{.u}}{{end}}<a href="{{template "u" .}}">x</a>',
        '<a href="{{block "u" .}}{{.u}}{{end}}">x</a>',
        '<iframe srcdoc="{{.x}}"></iframe>',
        "<{{.tag}}>",
        '<div {{.attr}}="{{.val}}">',
    ]) {
        expectTemplateError(() => render(bad, {}, opts), "PARSE");
    }

    const badSet = createTemplateSet(opts).add("u", "{{.u}}");
    expectTemplateError(() => badSet.add("href_call", '<a href="{{template "u" .}}">x</a>'), "PARSE");

    // Conservative URL-start join across `if` without `else`
    assert.strictEquals(
        render('<a href="{{if .p}}/v2/{{end}}{{.u}}">x</a>', { p: false, u: "javascript:alert(1)" }, opts),
        '<a href="#ZgotmplZ">x</a>',
    );

    // escapeHtml marker isolation
    assert.strictEquals(
        render('<a href="{{ .u | escapeHtml }}">x</a>', { u: "javascript:alert(1)" }, opts),
        '<a href="#ZgotmplZ">x</a>',
    );
    assert.strictEquals(
        render("<div class={{ .c | escapeHtml }}></div>", { c: "x onmouseover=alert(1)" }, opts),
        "<div class=x&#32;onmouseover&#61;alert(1)></div>",
    );
    assert.strictEquals(
        render("<button onclick=\"f('{{ .x | escapeHtml }}')\">go</button>", { x: "'); alert(1)//" }, opts),
        "<button onclick=\"f('\\u0026#39;); alert(1)\\u002f\\u002f')\">go</button>",
    );

    // URL multi-action & leading HTML entity
    assert.strictEquals(
        render('<a href="{{.a}}{{.b}}">x</a>', { a: "", b: "javascript:alert(1)" }, opts),
        '<a href="#ZgotmplZ">x</a>',
    );
    assert.strictEquals(
        render('<a href="{{.a}}{{.b}}">x</a>', { a: "javascript", b: ":alert(1)" }, opts),
        '<a href="javascript#ZgotmplZ">x</a>',
    );
    assert.strictEquals(
        render('<a href="&Tab;{{.u}}">x</a>', { u: "javascript:alert(1)" }, opts),
        '<a href="&Tab;#ZgotmplZ">x</a>',
    );

    // HTML5 comments, <script> double-escape, bogus comment </1, and JS lexer edge cases
    assert.strictEquals(
        render('<!--><script>var x="{{.u}}";</script>', { u: '";alert(1)//' }, opts),
        '<!--><script>var x="\\u0022;alert(1)\\u002f\\u002f";</script>',
    );
    assert.strictEquals(
        render('<!-- a --!><script>var x="{{.u}}";</script>', { u: '";alert(1)//' }, opts),
        '<!-- a --!><script>var x="\\u0022;alert(1)\\u002f\\u002f";</script>',
    );
    assert.strictEquals(
        render('<!-- {{.c}} -->', { c: "-!-" }, opts),
        '<!-- &#45;&#33;&#45; -->',
    );
    assert.strictEquals(
        render('<!-- hello {{.name}} -->', { name: "world --> <script>" }, opts),
        '<!-- hello world &#45;&#45;&gt; &lt;script&gt; -->',
    );
    assert.strictEquals(
        render("<script><!-- <script> </script> var x={{.u}}; --></script>", { u: "alert(1)" }, opts),
        '<script><!-- <script> </script> var x="alert(1)"; --></script>',
    );
    assert.strictEquals(
        render("<button onclick=\"/* ' */ {{.x}}\">go</button>", { x: "alert(1)" }, opts),
        '<button onclick="/* \' */ &quot;alert(1)&quot;">go</button>',
    );
    assert.strictEquals(
        render('<button onclick="f(&#96;{{.x}}&#96;)">go</button>', { x: "${alert(1)}" }, opts),
        '<button onclick="f(&#96;\\u0024{alert(1)}&#96;)">go</button>',
    );
    assert.strictEquals(
        render('<script>var r=/\\"/; var x={{.u}};</script>', { u: "alert(1)" }, opts),
        '<script>var r=/\\"/; var x="alert(1)";</script>',
    );
    assert.strictEquals(
        render("<script>var s=`a ${ {{.u}} }`;</script>", { u: "alert(1)" }, opts),
        '<script>var s=`a ${ "alert(1)" }`;</script>',
    );
    assert.strictEquals(
        render('<script>var s=`a ${"`"}`; var x={{.u}};</script>', { u: "alert(1)" }, opts),
        '<script>var s=`a ${"`"}`; var x="alert(1)";</script>',
    );
    assert.strictEquals(
        render('<script>var x=1; <!-- "\n var y={{.u}};</script>', { u: "alert(1)" }, opts),
        '<script>var x=1; <!-- "\n var y="alert(1)";</script>',
    );
    assert.strictEquals(
        render('</1 title="><script>var x={{.u}};</script>">', { u: "alert(1)" }, opts),
        '</1 title="><script>var x="alert(1)";</script>">',
    );
});

test("htmlEscape: true fail-closes EOF non-Text states, comment substates, UTF-8/numeric entities, line continuations, JS slash ambiguity, and static URL schemes", () => {
    const opts = { htmlEscape: true };

    // 1. Non-Text EOF state check (CompiledTemplate & TemplateSet)
    for (const unclosed of [
        "<script>var x = {{.x}};",
        "<script>",
        "<!-- {{.x}}",
        '<div title="{{.x}}',
        "<style>p { color: {{.x}};",
        "<textarea>{{.x}}",
        "<title>{{.x}}",
    ]) {
        expectTemplateError(() => render(unclosed, { x: "1" }, opts), "PARSE");
    }

    const openSet = createTemplateSet(opts);
    expectTemplateError(() => openSet.add("open", "<script>"), "PARSE");
    expectTemplateError(() => openSet.add("defs", '{{define "open"}}<script>{{end}}'), "PARSE");
    expectTemplateError(() => openSet.add("blk", '{{block "open" .}}<script>{{end}}'), "PARSE");

    // 2. HTML5 comment substates, branch equality, and close-token lookahead
    for (const badComment of [
        '<!--{{if .x}}foo{{end}}><script>var a="{{.u}}";</script>',
        '<!--{{.c}}><script>var x="{{.u}}";</script>',
        '<!---{{.c}}><script>var x="{{.u}}";</script>',
        '<!-- a --{{.c}}><script>var x="{{.u}}";</script>',
        '<!-- a --!{{.c}}><script>var x="{{.u}}";</script>',
        "<!-- a -{{.c}}->",
        '<!-- -{{.c}}><script>var x="{{.u}}";</script>',
        "<!-- {{if .x}}--{{end}}> -->",
        "<!-- {{if .x}}foo{{end}}>",
        "<!-- {{.a}}{{if .x}}foo{{end}}>",
        "<!-- {{.c}}->",
        "<!-- {{.c}}!>",
        "<!-- {{- .c}} -->",
    ]) {
        expectTemplateError(() => render(badComment, {}, opts), "PARSE");
    }
    assert.strictEquals(
        render("<!-- {{if .x}}hello{{else}}world{{end}} -->", { x: true }, opts),
        "<!-- hello -->",
    );

    // 3. UTF-8 entity scan safety & WHATWG numeric/named HTML entities in `on*`
    assert.strictEquals(
        render('<button onclick="&aaaaaaaaaaaaaa\u00e9; f({{.x}})">go</button>', { x: "1);alert(1)//" }, opts),
        '<button onclick="&aaaaaaaaaaaaaa\u00e9; f(&quot;1);alert(1)\\u002f\\u002f&quot;)">go</button>',
    );
    assert.strictEquals(
        render('<button onclick="f(&#96+{{.x}}+&#96)">go</button>', { x: "${alert(1)}" }, opts),
        '<button onclick="f(&#96+\\u0024{alert(1)}+&#96)">go</button>',
    );
    assert.strictEquals(
        render('<button onclick="f(&#00000000000000000096;{{.x}}&#00000000000000000096;)">go</button>', { x: "${alert(1)}" }, opts),
        '<button onclick="f(&#00000000000000000096;\\u0024{alert(1)}&#00000000000000000096;)">go</button>',
    );
    assert.strictEquals(
        render("<button onclick=\"&#47* ' */ {{.x}}\">go</button>", { x: "alert(1)" }, opts),
        '<button onclick="&#47* \' */ &quot;alert(1)&quot;">go</button>',
    );
    assert.strictEquals(
        render('<button onclick="&#0000000000000000047;* &#39; */ {{.x}}">go</button>', { x: "alert(1)" }, opts),
        '<button onclick="&#0000000000000000047;* &#39; */ &quot;alert(1)&quot;">go</button>',
    );

    // 4. JS LineContinuation (`\<newline>` and `\<CR><LF>`) inside quoted strings
    assert.strictEquals(
        render('<script>var s = "\\\n{{.x}}";</script>', { x: '";alert(1)//' }, opts),
        '<script>var s = "\\\n\\u0022;alert(1)\\u002f\\u002f";</script>',
    );
    assert.strictEquals(
        render('<script>var s = "\\\r\n{{.x}}";</script>', { x: '";alert(1)//' }, opts),
        '<script>var s = "\\\r\n\\u0022;alert(1)\\u002f\\u002f";</script>',
    );

    // 5. Fail-closed JS regex-vs-division disambiguation & `++`/`--`
    for (const badJs of [
        '<script>{{if .c}}var x=1{{else}}var x=1+{{end}} /a"/; var y={{.u}}+"";</script>',
        '<script>var x={} /a"/; var y={{.u}}+"";</script>',
        '<script>if (1) /"/; var y={{.u}};</script>',
        '<script>var s = "\\{{.x}}";</script>',
        "<script>var s = `${{.x}}`;</script>",
        '<script>var r = /{{.x}}/; var y = {{.u}};</script>',
    ]) {
        expectTemplateError(() => render(badJs, {}, opts), "PARSE");
    }
    assert.strictEquals(
        render('<script>var y = x++ / b + "{{.x}}";</script>', { x: '";alert(1)//' }, opts),
        '<script>var y = x++ / b + "\\u0022;alert(1)\\u002f\\u002f";</script>',
    );
    assert.strictEquals(
        render('<script>var x = obj.return /a"/; var y={{.u}}+"";</script>', { u: '";alert(1)//' }, opts),
        '<script>var x = obj.return /a"/; var y=\\u0022;alert(1)\\u002f\\u002f+"";</script>',
    );
    assert.strictEquals(
        render("<script>var x = obj.return / a; var y={{.u}};</script>", { u: "alert(1)" }, opts),
        '<script>var x = obj.return / a; var y="alert(1)";</script>',
    );

    // 6. URL static unsafe schemes & static `:` after dynamic prefix
    for (const badUrlTpl of [
        '<a href="javascript:{{.u}}">x</a>',
        '<a href="vbscript:{{.u}}">x</a>',
        '<a href="data:{{.u}}">x</a>',
        '<a href="java&#115;cript:{{.u}}">x</a>',
        '<a href="{{.a}}:{{.b}}">x</a>',
        '<a href="{{.a}}:alert(1)">x</a>',
        '<a href="{{.a}}&#58;alert(1)">x</a>',
        '<a href="{{.a}}&colon;alert(1)">x</a>',
    ]) {
        expectTemplateError(() => render(badUrlTpl, {}, opts), "PARSE");
    }

    // 7. RCDATA isolation
    assert.strictEquals(
        render("<textarea><script></textarea><a href={{.u}}>", { u: "javascript:alert(1)" }, opts),
        "<textarea><script></textarea><a href=#ZgotmplZ>",
    );

    // 8. Consecutive comments trim/coalescing, decimal dot vs member access, <noscript> RawText, labeled break ASI, non-ASCII JS ident
    assert.strictEquals(
        render('<!-- {{/* a */}}{{- /* b */}}><script>var a="{{.u}}";</script>-->', { u: '";alert(1)//' }, opts),
        '<!--><script>var a="\\u0022;alert(1)\\u002f\\u002f";</script>-->',
    );
    assert.strictEquals(
        render("<script>/ {{/* a */}}{{- /* b */}}* / ' */ var y = {{.u}}; /* ' */</script>", { u: "alert(1)" }, opts),
        '<script>/* / \' */ var y = "alert(1)"; /* \' */</script>',
    );
    assert.strictEquals(
        render('<script>var x = 1.\ntypeof /"/; var y = {{.u}}; // "\n</script>', { u: "alert(1)" }, opts),
        '<script>var x = 1.\ntypeof /"/; var y = "alert(1)"; // "\n</script>',
    );
    assert.strictEquals(
        render('<noscript><div title="</noscript><script>var x={{.u}};</script>"></noscript>', { u: "alert(1)" }, opts),
        '<noscript><div title="</noscript><script>var x="alert(1)";</script>"></noscript>',
    );
    assert.strictEquals(
        render("<script>l: { break l\n/a'b/; } var y = {{.x}};</script>", { x: "alert(1)" }, opts),
        "<script>l: { break l\n/a'b/; } var y = \"alert(1)\";</script>",
    );
    assert.strictEquals(
        render('<script>var x = \u00e9return / 2 + "{{.u}}";</script>', { u: '";alert(1);//' }, opts),
        '<script>var x = \u00e9return / 2 + "\\u0022;alert(1);\\u002f\\u002f";</script>',
    );
});
