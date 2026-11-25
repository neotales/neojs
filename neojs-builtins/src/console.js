(function installNeoConsole(host) {
    "use strict";

    // Shared `console` formatter and method installer for both V8 (`neojs`) and
    // QuickJS (`neojs-lite`).
    //
    // `log`, `debug`, and `info` write to stdout (`isError = false`); `warn`
    // and `error` write to stderr (`isError = true`). Strings print unchanged,
    // bigints print with a trailing `n`, genuine typed arrays print their
    // intrinsic type and up to ten elements, and objects format as indented
    // JSON when serializable, falling back safely without invoking tampered
    // prototypes or `Array[Symbol.species]`.
    //
    // Intrinsics are captured at installation time so later global mutation
    // cannot alter formatting.
    const makeTypeError = TypeError;
    const getPrototypeOf = Object.getPrototypeOf;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const mathMin = Math.min;
    const stringify = JSON.stringify;
    const toNativeString = String;
    const objectToString = Function.prototype.call.bind(Object.prototype.toString);

    const typedArrayPrototype = getPrototypeOf(Uint8Array.prototype);
    const typedArrayLengthOf = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayPrototype, "length").get,
    );
    const typedArrayTagOf = Function.prototype.call.bind(
        getOwnPropertyDescriptor(typedArrayPrototype, Symbol.toStringTag).get,
    );

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

    const typedArrayTag = (value) => {
        try {
            return typedArrayTagOf(value);
        } catch {
            return undefined;
        }
    };

    const safeString = (value) => {
        try {
            return toNativeString(value);
        } catch {
            try {
                return objectToString(value);
            } catch {
                return "[unprintable]";
            }
        }
    };

    const format = (value) => {
        if (typeof value === "string") return value;
        if (typeof value === "bigint") return `${value}n`;
        if (typeof value === "symbol" || typeof value === "function") {
            return safeString(value);
        }
        const tag = typedArrayTag(value);
        if (tag !== undefined) {
            const length = typedArrayLengthOf(value);
            const count = mathMin(length, 10);
            let elements = "";
            for (let index = 0; index < count; index += 1) {
                if (index > 0) elements += ", ";
                elements += value[index];
            }
            return `${tag}(${length}) [${elements}]`;
        }
        try {
            const json = stringify(value, null, 2);
            return json === undefined ? safeString(value) : json;
        } catch {
            return safeString(value);
        }
    };

    const writer = (isError) => (...args) => {
        let output = "";
        for (let index = 0; index < args.length; index += 1) {
            if (index > 0) output += " ";
            output += format(args[index]);
        }
        output += "\n";
        print(output, isError);
    };

    target.log = writer(false);
    target.debug = writer(false);
    target.info = writer(false);
    target.warn = writer(true);
    target.error = writer(true);
    return target;
})
