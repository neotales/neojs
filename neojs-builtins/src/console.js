(function installNeoConsole(host) {
    "use strict";

    // Shared `console` formatter and method installer for both V8 (`neojs`) and
    // QuickJS (`neojs-lite`).
    //
    // `log`, `debug`, and `info` write to stdout (`isError = false`); `warn`
    // and `error` write to stderr (`isError = true`). Strings print unchanged,
    // bigints print with a trailing `n`, typed arrays print their type and up
    // to ten elements, and objects format as indented JSON when serializable.
    //
    // Intrinsics are captured at installation time so later global mutation
    // cannot alter formatting.
    const makeTypeError = TypeError;
    const mathMin = Math.min;
    const isView = ArrayBuffer.isView;
    const NativeDataView = typeof DataView === "function" ? DataView : null;
    const stringify = JSON.stringify;
    const toNativeString = String;
    const arrayMap = Function.prototype.call.bind(Array.prototype.map);
    const arrayJoin = Function.prototype.call.bind(Array.prototype.join);

    if (
        !host ||
        typeof host.print !== "function" ||
        !host.target ||
        typeof host.target !== "object"
    ) {
        throw new makeTypeError(
            "neo:console host requires a print function and target object",
        );
    }

    const print = host.print;
    const target = host.target;

    const format = (value) => {
        if (typeof value === "string") return value;
        if (typeof value === "bigint") return `${value}n`;
        if (typeof value === "symbol" || typeof value === "function") {
            return toNativeString(value);
        }
        if (isView(value) && (NativeDataView === null || !(value instanceof NativeDataView))) {
            const count = mathMin(value.length, 10);
            const elements = [];
            for (let index = 0; index < count; index += 1) {
                elements[index] = value[index];
            }
            return `${value.constructor.name}(${value.length}) [${arrayJoin(elements, ", ")}]`;
        }
        try {
            const json = stringify(value, null, 2);
            return json === undefined ? toNativeString(value) : json;
        } catch {
            return toNativeString(value);
        }
    };

    const writer = (isError) => (...args) => {
        print(`${arrayJoin(arrayMap(args, format), " ")}\n`, isError);
    };

    target.log = writer(false);
    target.debug = writer(false);
    target.info = writer(false);
    target.warn = writer(true);
    target.error = writer(true);
    return target;
})
