    const template = (() => {
        const templateOps = Deno.core.ops;
        const opTemplateRender = templateOps.op_neo_template_render;
        const opTemplateCompile = templateOps.op_neo_template_compile;
        const opTemplateCompiledRender = templateOps.op_neo_template_compiled_render;
        const opTemplateSetCreate = templateOps.op_neo_template_set_create;
        const opTemplateSetAdd = templateOps.op_neo_template_set_add;
        const opTemplateSetRender = templateOps.op_neo_template_set_render;
        const opTemplateSetNames = templateOps.op_neo_template_set_names;
        const opTemplateEscapeHtml = templateOps.op_neo_template_escape_html;
        const opTemplateRelease = templateOps.op_neo_template_release;
        const opTemplateSetRelease = templateOps.op_neo_template_set_release;
        const finalizationRegister = Function.prototype.call.bind(
            FinalizationRegistry.prototype.register,
        );
        const compiledRegistry = new FinalizationRegistry((id) => {
            opTemplateRelease(id);
        });
        const setRegistry = new FinalizationRegistry((id) => {
            opTemplateSetRelease(id);
        });
        const templateToken = Symbol("neojs.template.Template");
        const templateSetToken = Symbol("neojs.template.TemplateSet");

        function throwTemplate(response) {
            let error;
            if (response.name === "TypeError") {
                error = new makeTypeError(response.message);
            } else if (response.name === "RangeError") {
                error = new makeRangeError(response.message);
            } else {
                error = new makeError(response.message);
                defineProperty(error, "name", {
                    __proto__: null,
                    value: "TemplateError",
                    writable: true,
                    enumerable: false,
                    configurable: true,
                });
            }
            defineProperty(error, "code", {
                __proto__: null,
                value: response.code,
                writable: true,
                enumerable: true,
                configurable: true,
            });
            throw error;
        }

        function unwrapValue(response) {
            if (!response.ok) throwTemplate(response);
            return response.value;
        }

        function unwrapUnit(response) {
            if (!response.ok) throwTemplate(response);
        }

        class Template {
            #id;

            constructor(token, id) {
                if (token !== templateToken) {
                    throw new makeTypeError(
                        "Template objects cannot be constructed directly; use compile()",
                    );
                }
                this.#id = id;
                finalizationRegister(compiledRegistry, this, id);
                freeze(this);
            }

            render(data) {
                return unwrapValue(opTemplateCompiledRender(this.#id, data));
            }
        }
        freeze(Template.prototype);
        freeze(Template);

        class TemplateSet {
            #id;

            constructor(token, id) {
                if (token !== templateSetToken) {
                    throw new makeTypeError(
                        "TemplateSet objects cannot be constructed directly; use createTemplateSet()",
                    );
                }
                this.#id = id;
                finalizationRegister(setRegistry, this, id);
                freeze(this);
            }

            add(name, source) {
                unwrapUnit(opTemplateSetAdd(this.#id, name, source));
                return this;
            }

            render(name, data) {
                return unwrapValue(opTemplateSetRender(this.#id, name, data));
            }

            names() {
                return unwrapValue(opTemplateSetNames(this.#id));
            }
        }
        freeze(TemplateSet.prototype);
        freeze(TemplateSet);

        function render(source, data, options) {
            return unwrapValue(opTemplateRender(source, data, options));
        }

        function compile(source, options) {
            const id = unwrapValue(opTemplateCompile(source, options));
            return new Template(templateToken, id);
        }

        function createTemplateSet(options) {
            const id = unwrapValue(opTemplateSetCreate(options));
            return new TemplateSet(templateSetToken, id);
        }

        function escapeHtml(input) {
            return unwrapValue(opTemplateEscapeHtml(input));
        }

        return freeze({
            render,
            compile,
            createTemplateSet,
            escapeHtml,
        });
    })();