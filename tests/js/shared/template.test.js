import test from "neo:test";
import assert from "neo:assert";
import templateDefault, * as templateNs from "neo:template";
import { compile, createTemplateSet, escapeHtml, render } from "neo:template";

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

test("neo:template registry, named exports, and default export share frozen identity", async () => {
    const list = Neo.getBuiltInModulesList();
    assert.ok(list.includes("neo:template"), "neo:template missing from Neo.getBuiltInModulesList()");
    assert.ok(!list.includes("template"), "bare template must not be listed");
    assert.strictEquals(Neo.getBuiltInModule("template"), undefined);

    const registry = Neo.getBuiltInModule("neo:template");
    assert.ok(registry !== undefined && typeof registry === "object" && registry !== null);
    assert.strictEquals(Neo.getBuiltInModule("neo:template"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(templateDefault, registry);
    assert.ok(templateNs !== registry, "module namespace must be distinct from registry");

    const dyn = await import("neo:template");
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
    assert.strictEquals(render("Hello, {{.name}}!", { name: "Neo" }), "Hello, Neo!");
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
        set.render("home", { name: "Neo", tag: "v1" }),
        "<html><title>Home</title><body><h1>Hi Neo</h1><span>v1</span></body></html>",
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
