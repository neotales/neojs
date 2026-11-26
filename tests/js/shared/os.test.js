import test from "vael:test";
import assert from "vael:assert";
import * as os from "vael:os";
import { arch, platform } from "vael:os";

// Documented Deno.build.os and normalized OCI/Go-style target arch vocabulary.
const DENO_OS_VOCABULARY = new Set([
    "darwin",
    "linux",
    "windows",
    "freebsd",
    "netbsd",
    "aix",
    "solaris",
    "illumos",
    "android",
]);

const NORMALIZED_ARCH_VOCABULARY = new Set([
    "amd64",
    "arm64",
    "386",
    "arm",
    "riscv64",
    "ppc64le",
    "ppc64",
    "s390x",
    "loong64",
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

test("vael:os exposes non-empty strings in normalized target vocabulary", () => {
    assert.strictEquals(typeof os.platform, "string");
    assert.strictEquals(typeof os.arch, "string");
    assert.ok(os.platform.length > 0, "platform must not be empty");
    assert.ok(os.arch.length > 0, "arch must not be empty");

    // `platform` uses Deno's target vocabulary (`Deno.build.os`), not Rust's
    // `"macos"` or Node's `"win32"`. `arch` uses normalized OCI/Go-style short
    // names (`"amd64"`, `"arm64"`, `"386"`, etc.), not Rust's
    // `"x86_64"`/`"aarch64"` or Node's `"x64"`/`"ia32"`.
    assert.ok(!["macos", "win32"].includes(os.platform), `unexpected non-Deno OS name: ${os.platform}`);
    assert.ok(
        !["x86_64", "aarch64", "x64", "ia32"].includes(os.arch),
        `unexpected non-normalized arch name: ${os.arch}`,
    );
    assert.ok(DENO_OS_VOCABULARY.has(os.platform), `unexpected Deno OS constant: ${os.platform}`);
    assert.ok(
        NORMALIZED_ARCH_VOCABULARY.has(os.arch),
        `unexpected normalized ARCH constant: ${os.arch}`,
    );
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
