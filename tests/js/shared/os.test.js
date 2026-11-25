import test from "vael:test";
import assert from "vael:assert";
import * as os from "vael:os";
import { arch, platform } from "vael:os";

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

test("vael:os registry, list, and module namespace share frozen identity", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:os"), `vael:os missing from list: ${JSON.stringify(list)}`);
    assert.ok(!list.includes("os"), "bare os must not be listed");

    const registry = Vael.getBuiltInModule("vael:os");
    const dynamic = await import("vael:os");
    const again = await import("vael:os");

    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Vael.getBuiltInModule("vael:os"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(Object.isExtensible(os), false);
    assert.ok(os !== registry, "module namespace must be distinct from registry");
    assert.strictEquals(dynamic, again);
    assert.strictEquals(dynamic, os);
    assert.ok(!("default" in os), "vael:os must not have a default export");

    const exported = Object.keys(os).sort();
    const registered = Object.keys(registry).sort();
    assert.equals(exported, ["arch", "platform"]);
    assert.equals(registered, ["arch", "platform"]);

    assert.strictEquals(platform, os.platform);
    assert.strictEquals(arch, os.arch);
    assert.strictEquals(os.platform, registry.platform);
    assert.strictEquals(os.arch, registry.arch);
});

test("vael:os exposes non-empty strings in Rust target vocabulary", () => {
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

test("vael:os resists mutation and rejects bare specifiers", async () => {
    const registry = Vael.getBuiltInModule("vael:os");

    for (const name of ["platform", "arch"]) {
        assert.throws(() => {
            os[name] = "replaced";
        }, TypeError);
        assert.throws(() => {
            Object.defineProperty(registry, name, { value: "replaced" });
        }, TypeError);
    }

    assert.strictEquals(Vael.getBuiltInModule("os"), undefined);
    await assert.rejects(() => import("os"));

    for (const name of ["os", "__vaelOs", "__VAEL_OS_PLATFORM__", "__VAEL_OS_ARCH__"]) {
        assert.ok(!(name in globalThis), `${name} leaked onto globalThis`);
    }
});
