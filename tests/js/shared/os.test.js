import test from "neo:test";
import assert from "neo:assert";
import * as os from "neo:os";
import { arch, platform } from "neo:os";

// Documented Rust std::env::consts::OS and std::env::consts::ARCH values.
const RUST_OS_VOCABULARY = new Set([
    "linux",
    "macos",
    "windows",
    "freebsd",
    "netbsd",
    "openbsd",
    "dragonfly",
    "android",
    "ios",
    "illumos",
    "solaris",
    "fuchsia",
    "haiku",
    "redox",
    "emscripten",
    "wasi",
    "aix",
    "hurd",
    "nto",
    "tvos",
    "visionos",
    "watchos",
]);

const RUST_ARCH_VOCABULARY = new Set([
    "x86_64",
    "x86",
    "aarch64",
    "arm",
    "riscv64",
    "riscv32",
    "powerpc64",
    "powerpc",
    "s390x",
    "loongarch64",
    "wasm32",
    "wasm64",
    "mips",
    "mips64",
    "sparc",
    "sparc64",
]);

test("neo:os registry, list, and module namespace share frozen identity", async () => {
    const list = Neo.getBuiltInModulesList();
    assert.ok(list.includes("neo:os"), `neo:os missing from list: ${JSON.stringify(list)}`);
    assert.ok(!list.includes("os"), "bare os must not be listed");

    const registry = Neo.getBuiltInModule("neo:os");
    const dynamic = await import("neo:os");
    const again = await import("neo:os");

    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Neo.getBuiltInModule("neo:os"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(Object.isExtensible(os), false);
    assert.ok(os !== registry, "module namespace must be distinct from registry");
    assert.strictEquals(dynamic, again);
    assert.strictEquals(dynamic, os);
    assert.ok(!("default" in os), "neo:os must not have a default export");

    const exported = Object.keys(os).sort();
    const registered = Object.keys(registry).sort();
    assert.equals(exported, ["arch", "platform"]);
    assert.equals(registered, ["arch", "platform"]);

    assert.strictEquals(platform, os.platform);
    assert.strictEquals(arch, os.arch);
    assert.strictEquals(os.platform, registry.platform);
    assert.strictEquals(os.arch, registry.arch);
});

test("neo:os exposes non-empty strings in Rust target vocabulary", () => {
    assert.strictEquals(typeof os.platform, "string");
    assert.strictEquals(typeof os.arch, "string");
    assert.ok(os.platform.length > 0, "platform must not be empty");
    assert.ok(os.arch.length > 0, "arch must not be empty");

    // Uses Rust's std::env::consts vocabulary, not Node's process.platform/arch.
    assert.ok(!["darwin", "win32"].includes(os.platform), `unexpected Node OS name: ${os.platform}`);
    assert.ok(!["x64", "arm64", "ia32"].includes(os.arch), `unexpected Node arch name: ${os.arch}`);
    assert.ok(RUST_OS_VOCABULARY.has(os.platform), `unexpected Rust OS constant: ${os.platform}`);
    assert.ok(RUST_ARCH_VOCABULARY.has(os.arch), `unexpected Rust ARCH constant: ${os.arch}`);
});

test("neo:os resists mutation and rejects bare specifiers", async () => {
    const registry = Neo.getBuiltInModule("neo:os");

    for (const name of ["platform", "arch"]) {
        assert.throws(() => {
            os[name] = "replaced";
        }, TypeError);
        assert.throws(() => {
            Object.defineProperty(registry, name, { value: "replaced" });
        }, TypeError);
    }

    assert.strictEquals(Neo.getBuiltInModule("os"), undefined);
    await assert.rejects(() => import("os"));

    for (const name of ["os", "__neoOs", "__NEO_OS_PLATFORM__", "__NEO_OS_ARCH__"]) {
        assert.ok(!(name in globalThis), `${name} leaked onto globalThis`);
    }
});
