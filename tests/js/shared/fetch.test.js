import test from "vael:test";
import assert from "vael:assert";
import { tmpdir } from "node:os";
import { join } from "vael:path";
import { exists, mkdir, remove } from "vael:fs";
import { v4 } from "vael:uuid";
import * as fetchMod from "vael:fetch";
import { FetchError, downloadSync, fetchSync } from "vael:fetch";

const EXPECTED_LITE_EXPORTS = ["FetchError", "downloadSync", "fetchSync"];
const EXPECTED_V8_EXPORTS = ["FetchError", "download", "downloadSync", "fetch", "fetchSync"];

function withTempDir(fn) {
    const root = join(tmpdir(), `vaeljs-fetch-test-${v4()}`);
    mkdir(root, { recursive: true });
    let threw = false;
    try {
        return fn(root);
    } catch (err) {
        threw = true;
        throw err;
    } finally {
        if (exists(root)) {
            if (threw) {
                try {
                    remove(root, { recursive: true });
                } catch {
                    // Never mask the original failure thrown by `fn`.
                }
            } else {
                remove(root, { recursive: true });
            }
        }
    }
}

function assertFetchError(fn, expectedCode) {
    let caught;
    try {
        fn();
    } catch (err) {
        caught = err;
    }
    assert.ok(caught instanceof Error, `expected Error, got ${caught}`);
    assert.ok(caught instanceof FetchError, `expected FetchError, got ${caught?.constructor?.name}`);
    assert.strictEquals(caught.name, "FetchError");
    assert.strictEquals(
        caught.code,
        expectedCode,
        `expected ${expectedCode}, got ${caught.code} (${caught.message})`,
    );
    assert.strictEquals(typeof caught.message, "string");
    assert.ok(caught.message.length > 0, "FetchError message must not be empty");
    return caught;
}

async function assertFetchRejects(fn, expectedCode) {
    let caught;
    try {
        await fn();
    } catch (err) {
        caught = err;
    }
    assert.ok(caught instanceof Error, `expected Error rejection, got ${caught}`);
    assert.ok(caught instanceof FetchError, `expected FetchError, got ${caught?.constructor?.name}`);
    assert.strictEquals(caught.name, "FetchError");
    assert.strictEquals(
        caught.code,
        expectedCode,
        `expected ${expectedCode}, got ${caught.code} (${caught.message})`,
    );
    return caught;
}

test("vael:fetch registry, module namespace, and global isolation", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:fetch"), "vael:fetch missing from Vael.getBuiltInModulesList()");
    assert.ok(!list.includes("fetch"), "bare fetch must not be listed");

    assert.strictEquals(typeof globalThis.fetch, "undefined", "globalThis.fetch must not exist");

    const reg = Vael.getBuiltInModule("vael:fetch");
    assert.ok(reg !== undefined && typeof reg === "object");
    assert.strictEquals(Vael.getBuiltInModule("vael:fetch"), reg);
    assert.strictEquals(Vael.getBuiltInModule("fetch"), undefined);
    assert.strictEquals(Object.isFrozen(reg), true);
    assert.strictEquals(Object.isExtensible(fetchMod), false);
    assert.ok(!("default" in fetchMod), "vael:fetch must not have a default export");

    const dyn = await import("vael:fetch");
    assert.strictEquals(dyn, fetchMod);
    assert.ok(fetchMod !== reg, "module namespace must be distinct from registry");

    const expected = Vael.engine === "vael" ? EXPECTED_V8_EXPORTS : EXPECTED_LITE_EXPORTS;
    assert.equals(Object.keys(fetchMod).sort(), expected);
    assert.equals(Object.keys(reg).sort(), expected);

    assert.strictEquals(FetchError, reg.FetchError);
    assert.strictEquals(fetchSync, reg.fetchSync);
    assert.strictEquals(downloadSync, reg.downloadSync);
    assert.strictEquals(fetchMod.FetchError, reg.FetchError);
    assert.strictEquals(fetchMod.fetchSync, reg.fetchSync);
    assert.strictEquals(fetchMod.downloadSync, reg.downloadSync);
    assert.strictEquals(dyn.fetchSync, reg.fetchSync);
    assert.strictEquals(dyn.downloadSync, reg.downloadSync);
});

test("FetchError prototype, name, and code", () => {
    assert.ok(FetchError.prototype instanceof Error);
    const err = new FetchError("VAEL_FETCH_INVALID_URL", "invalid URL");
    assert.ok(err instanceof Error);
    assert.ok(err instanceof FetchError);
    assert.strictEquals(err.name, "FetchError");
    assert.strictEquals(err.code, "VAEL_FETCH_INVALID_URL");
    assert.strictEquals(err.message, "invalid URL");
});

test("fetchSync blocks SSRF loopback, link-local metadata, non-HTTP schemes, and embedded userinfo", () => {
    for (const ssrfUrl of [
        "http://127.0.0.1:1/",
        "http://[::1]:1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://localhost:1/",
        "http://10.0.0.1/",
        "http://192.168.1.1/",
    ]) {
        assertFetchError(() => fetchSync(ssrfUrl), "VAEL_FETCH_SSRF_BLOCKED");
    }

    assertFetchError(() => fetchSync("file:///etc/passwd"), "VAEL_FETCH_INVALID_URL");
    assertFetchError(() => fetchSync("ftp://example.com/resource"), "VAEL_FETCH_INVALID_URL");

    const userinfoErr = assertFetchError(
        () => fetchSync("https://user:pass@example.com/secret?token=xyz"),
        "VAEL_FETCH_INVALID_URL",
    );
    assert.ok(!userinfoErr.message.includes("user"), "error leaked username");
    assert.ok(!userinfoErr.message.includes("pass"), "error leaked password");
    assert.ok(!userinfoErr.message.includes("xyz"), "error leaked query token");
});

test("fetchSync rejects CRLF headers, body+json conflict, and invalid options", () => {
    assertFetchError(
        () =>
            fetchSync("https://example.com/", {
                headers: { "X-Test": "value\r\nX-Injected: bad" },
            }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () =>
            fetchSync("https://example.com/", {
                headers: [["Bad\nHeader", "value"]],
            }),
        "VAEL_FETCH_INVALID_OPTION",
    );

    assertFetchError(
        () =>
            fetchSync("https://example.com/", {
                method: "POST",
                body: "raw-body",
                json: { a: 1 },
            }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () =>
            fetchSync("https://example.com/", {
                method: "GET",
                body: "not-allowed-on-get",
            }),
        "VAEL_FETCH_INVALID_OPTION",
    );

    assertFetchError(
        () => fetchSync("https://example.com/", { timeoutMs: 0 }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () => fetchSync("https://example.com/", { timeoutMs: -5 }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () => fetchSync("https://example.com/", { timeoutMs: 600_001 }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () => fetchSync("https://example.com/", { method: "TRACE" }),
        "VAEL_FETCH_INVALID_OPTION",
    );
    assertFetchError(
        () => fetchSync("https://example.com/", { method: "GET\r\n" }),
        "VAEL_FETCH_INVALID_OPTION",
    );
});

test("downloadSync enforces SSRF blocking, URL validation, and SHA-256 option validation", () => {
    withTempDir((dir) => {
        const dest = join(dir, "artifact.bin");

        assertFetchError(() => downloadSync("http://127.0.0.1:1/", dest), "VAEL_FETCH_SSRF_BLOCKED");
        assertFetchError(() => downloadSync("http://[::1]:1/", dest), "VAEL_FETCH_SSRF_BLOCKED");
        assertFetchError(
            () => downloadSync("http://169.254.169.254/latest/meta-data/", dest),
            "VAEL_FETCH_SSRF_BLOCKED",
        );
        assertFetchError(() => downloadSync("file:///etc/passwd", dest), "VAEL_FETCH_INVALID_URL");
        assertFetchError(
            () => downloadSync("https://user:pass@example.com/file.bin", dest),
            "VAEL_FETCH_INVALID_URL",
        );

        assertFetchError(
            () =>
                downloadSync("https://example.com/file.bin", dest, {
                    expectedSha256: "not-a-valid-sha256",
                }),
            "VAEL_FETCH_INVALID_OPTION",
        );
        assertFetchError(
            () =>
                downloadSync("https://example.com/file.bin", dest, {
                    expectedSha256: "abcd",
                }),
            "VAEL_FETCH_INVALID_OPTION",
        );
        assertFetchError(
            () =>
                downloadSync("https://example.com/file.bin", dest, {
                    timeoutMs: 0,
                }),
            "VAEL_FETCH_INVALID_OPTION",
        );

        assert.strictEquals(exists(dest), false, "failed download must not create destination file");
    });
});

if (Vael.engine === "vael") {
    test("V8 async fetch and download reject SSRF and invalid options", async () => {
        await assertFetchRejects(() => fetchMod.fetch("http://127.0.0.1:1/"), "VAEL_FETCH_SSRF_BLOCKED");
        await assertFetchRejects(
            () => fetchMod.fetch("http://169.254.169.254/latest/meta-data/"),
            "VAEL_FETCH_SSRF_BLOCKED",
        );
        await assertFetchRejects(
            () => fetchMod.fetch("file:///etc/passwd"),
            "VAEL_FETCH_INVALID_URL",
        );
        await assertFetchRejects(
            () =>
                fetchMod.fetch("https://example.com/", {
                    method: "POST",
                    body: "x",
                    json: { x: 1 },
                }),
            "VAEL_FETCH_INVALID_OPTION",
        );

        const dir = join(tmpdir(), `vaeljs-fetch-async-${v4()}`);
        mkdir(dir, { recursive: true });
        try {
            const dest = join(dir, "dl.bin");
            await assertFetchRejects(
                () =>
                    fetchMod.download("https://example.com/dl.bin", dest, {
                        expectedSha256: "invalid",
                    }),
                "VAEL_FETCH_INVALID_OPTION",
            );
            assert.strictEquals(exists(dest), false);
        } finally {
            remove(dir, { recursive: true });
        }
    });
}
