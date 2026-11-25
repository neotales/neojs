// The `template` built-in module as seen by `import ... from "vael:template"`.
//
// The implementation lives in `vael.js`, which the runtime evaluates once per
// realm, and calls private host ops registered by `vaeljs-core/src/template.rs`.
// This file re-exports the frozen functions from the public `template` module
// object and exposes that frozen module object as the default export, so both
// paths share identity.
const template = Vael.getBuiltInModule("vael:template");

export const render = template.render;
export const compile = template.compile;
export const createTemplateSet = template.createTemplateSet;
export const escapeHtml = template.escapeHtml;
export default template;
