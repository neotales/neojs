// The `console` built-in module as seen by
// `import ... from "vael:console"`.
//
// The default export is the console object the runtime installed: the object
// `globalThis.console` named when the built-in registry was installed. The five
// methods vaeljs defines write to stdout (`log`, `info`, `debug`) and to stderr
// (`warn`, `error`), exactly as the global methods do. On V8 the object may
// also carry whatever other properties the engine's global console has; those
// are not part of vaeljs's documented surface.
//
// The named exports are the five method values read from that object when this
// module is evaluated, captured as constants. A method replaced before that
// first evaluation is what the named export captures; a replacement made later
// is not followed by the named binding. Replacing a property on the console
// object later is visible through the default export and
// `Vael.getBuiltInModule("vael:console")`, because they are the same object.
// Replacing the whole `globalThis.console` property does not change the
// captured object: this module reads the registry, not the global.
//
// The console object is not frozen. Unlike `chars`, this module's default
// export is the live console object the runtime installed, and freezing it
// would change existing console behaviour and break code that replaces a
// method. Neither this module nor the global console is a permission, sandbox,
// or isolation boundary.

// Read the registry rather than `globalThis.console`. The registry holds the
// console object the runtime installed, so replacing the global property before
// this module is evaluated cannot swap the module's default export.
const consoleObject = Vael.getBuiltInModule("vael:console");

export default consoleObject;

export const log = consoleObject.log;
export const debug = consoleObject.debug;
export const info = consoleObject.info;
export const warn = consoleObject.warn;
export const error = consoleObject.error;
