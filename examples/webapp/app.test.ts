import assert from "vael:assert";
import { type LogEntry, serve, type Server } from "vael:http";
import { dirname, join } from "vael:path";
import test from "vael:test";
import { fileURLToPath } from "node:url";
import { createApp } from "./app.ts";
import { openTaskDb, type Task } from "./db.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATIC_DIR = join(HERE, "public");
const UUID_V7_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

async function setupApp() {
  const db = await openTaskDb(":memory:");
  const logs: LogEntry[] = [];
  const app = createApp({
    db,
    staticDir: STATIC_DIR,
    logSink: (entry) => logs.push(entry),
  });
  return { db, app, logs };
}

function extractCookiePair(setCookieHeader: string): string {
  const firstPart = setCookieHeader.split(";")[0] ?? "";
  return firstPart.trim();
}

test("SSR GET / renders layout, Sprig helpers, and escapes XSS payloads", async () => {
  const { db, app } = await setupApp();
  try {
    const emptyRes = await app.mux.fetch("/");
    assert.strictEquals(emptyRes.status, 200);
    assert.strictEquals(
      emptyRes.header("content-type"),
      "text/html; charset=utf-8",
    );
    const emptyHtml = emptyRes.text();
    assert.ok(emptyHtml.includes("Vaeljs Task Board"));
    assert.ok(emptyHtml.includes("FILTER: ALL (0)"));
    assert.ok(emptyHtml.includes('href="/assets/style.css"'));

    const xssPayload = `<script>alert("xss")</script>`;
    const createRes = await app.mux.fetch("/api/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: `  ${xssPayload}  ` }),
    });
    assert.strictEquals(createRes.status, 201);

    const listRes = await app.mux.fetch("/?status=open");
    assert.strictEquals(listRes.status, 200);
    const htmlBody = listRes.text();
    assert.ok(htmlBody.includes("FILTER: OPEN (1)"));
    assert.ok(!htmlBody.includes(xssPayload), "raw <script> tag must not appear");
    assert.ok(
      htmlBody.includes("&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;"),
      "XSS payload must be HTML-escaped",
    );

    const doneRes = await app.mux.fetch("/?status=done");
    assert.strictEquals(doneRes.status, 200);
    assert.ok(doneRes.text().includes("FILTER: DONE (0)"));

    const badFilterRes = await app.mux.fetch("/?status=invalid");
    assert.strictEquals(badFilterRes.status, 400);
  } finally {
    await db.close();
  }
});

test("SSR form POST /tasks, signed flash cookie, toggle, and delete", async () => {
  const { db, app } = await setupApp();
  try {
    // Blank title is rejected with 400.
    const blankRes = await app.mux.fetch("/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: "title=+++",
    });
    assert.strictEquals(blankRes.status, 400);

    // Over-long title (> 200 chars) is rejected with 400 before DB write.
    const tooLongRes = await app.mux.fetch("/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: `title=${"a".repeat(201)}`,
    });
    assert.strictEquals(tooLongRes.status, 400);

    // 200-char multi-byte title succeeds and truncates flash cookie safely.
    const maxTitle = "🚀".repeat(200);
    const maxRes = await app.mux.fetch("/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: `title=${encodeURIComponent(maxTitle)}`,
    });
    assert.strictEquals(maxRes.status, 303);
    const maxFlashCookie = extractCookiePair(maxRes.headerValues("set-cookie")[0]);
    const maxHome = await app.mux.fetch("/", {
      headers: { Cookie: maxFlashCookie },
    });
    assert.strictEquals(maxHome.status, 200);
    assert.ok(maxHome.text().includes("…"));
    const maxList = await app.mux.fetch("/api/tasks");
    const maxTaskId = maxList.json<{ tasks: Task[] }>().tasks[0].id;
    await app.mux.fetch(`/tasks/${maxTaskId}/delete`, { method: "POST" });

    // Valid form submission creates task, returns 303, and sets signed flash cookie.
    const createRes = await app.mux.fetch("/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: "title=Ship+%3Cb%3Ewebapp%3C%2Fb%3E",
    });
    assert.strictEquals(createRes.status, 303);
    assert.strictEquals(createRes.header("location"), "/");
    const setCookies = createRes.headerValues("set-cookie");
    assert.strictEquals(setCookies.length, 1);
    assert.ok(setCookies[0].startsWith("flash="));
    assert.ok(setCookies[0].includes("HttpOnly"));
    assert.ok(setCookies[0].includes("SameSite=Lax"));

    const cookiePair = extractCookiePair(setCookies[0]);

    // Following redirect with valid signed cookie shows escaped flash & clears cookie.
    const homeWithFlash = await app.mux.fetch("/", {
      headers: { Cookie: cookiePair },
    });
    assert.strictEquals(homeWithFlash.status, 200);
    const homeHtml = homeWithFlash.text();
    assert.ok(
      homeHtml.includes(
        "Created task: Ship &lt;b&gt;webapp&lt;/b&gt;",
      ),
    );
    const clearCookies = homeWithFlash.headerValues("set-cookie");
    assert.strictEquals(clearCookies.length, 1);
    assert.ok(clearCookies[0].includes("Max-Age=0"));

    // Tampered flash cookie is ignored (HMAC fails) and cleared.
    const tamperedHome = await app.mux.fetch("/", {
      headers: { Cookie: "flash=Forged%20message.0000deadbeef" },
    });
    assert.strictEquals(tamperedHome.status, 200);
    assert.ok(!tamperedHome.text().includes("Forged message"));

    // Look up created task ID via JSON API to exercise SSR toggle and delete.
    const listApi = await app.mux.fetch("/api/tasks");
    const listBody = listApi.json<{ tasks: Task[]; count: number }>();
    assert.strictEquals(listBody.count, 1);
    const taskId = listBody.tasks[0].id;
    assert.ok(UUID_V7_RE.test(taskId));

    const toggleRes = await app.mux.fetch(`/tasks/${taskId}/toggle`, {
      method: "POST",
    });
    assert.strictEquals(toggleRes.status, 303);
    assert.strictEquals(toggleRes.header("location"), "/");

    const afterToggle = await app.mux.fetch(`/api/tasks/${taskId}`);
    assert.strictEquals(afterToggle.json<Task>().done, true);

    const deleteRes = await app.mux.fetch(`/tasks/${taskId}/delete`, {
      method: "POST",
    });
    assert.strictEquals(deleteRes.status, 303);

    const afterDelete = await app.mux.fetch(`/api/tasks/${taskId}`);
    assert.strictEquals(afterDelete.status, 404);

    // Toggle/delete on unknown ID return 404.
    const missingToggle = await app.mux.fetch("/tasks/does-not-exist/toggle", {
      method: "POST",
    });
    assert.strictEquals(missingToggle.status, 404);
    const missingDelete = await app.mux.fetch("/tasks/does-not-exist/delete", {
      method: "POST",
    });
    assert.strictEquals(missingDelete.status, 404);
  } finally {
    await db.close();
  }
});

test("JSON REST API full CRUD lifecycle and error handling", async () => {
  const { db, app } = await setupApp();
  try {
    const health = await app.mux.fetch("/api/health");
    assert.strictEquals(health.status, 200);
    assert.equals(health.json(), { ok: true, runtime: "vaeljs" });

    // POST /api/tasks -> 201
    const createdRes = await app.mux.fetch("/api/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "  Write VaelJS docs  " }),
    });
    assert.strictEquals(createdRes.status, 201);
    const created = createdRes.json<Task>();
    assert.ok(UUID_V7_RE.test(created.id));
    assert.strictEquals(created.title, "Write VaelJS docs");
    assert.strictEquals(created.done, false);

    // GET /api/tasks
    const listRes = await app.mux.fetch("/api/tasks");
    assert.strictEquals(listRes.status, 200);
    const listed = listRes.json<{ tasks: Task[]; count: number }>();
    assert.strictEquals(listed.count, 1);
    assert.equals(listed.tasks[0], created);

    // GET /api/tasks/{id}
    const getRes = await app.mux.fetch(`/api/tasks/${created.id}`);
    assert.strictEquals(getRes.status, 200);
    assert.equals(getRes.json<Task>(), created);

    // PATCH /api/tasks/{id}
    const patchRes = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Published VaelJS docs", done: true }),
    });
    assert.strictEquals(patchRes.status, 200);
    const updated = patchRes.json<Task>();
    assert.strictEquals(updated.title, "Published VaelJS docs");
    assert.strictEquals(updated.done, true);

    // DELETE /api/tasks/{id} -> 204
    const delRes = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "DELETE",
    });
    assert.strictEquals(delRes.status, 204);
    assert.strictEquals(delRes.text(), "");

    // 404 on missing ID
    const notFoundGet = await app.mux.fetch(`/api/tasks/${created.id}`);
    assert.strictEquals(notFoundGet.status, 404);
    assert.strictEquals(notFoundGet.json<{ code: string }>().code, "TASK_NOT_FOUND");

    const notFoundPatch = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ done: false }),
    });
    assert.strictEquals(notFoundPatch.status, 404);

    const notFoundDelete = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "DELETE",
    });
    assert.strictEquals(notFoundDelete.status, 404);

    // 400 on invalid payloads
    const badJson = await app.mux.fetch("/api/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{bad-json",
    });
    assert.strictEquals(badJson.status, 400);

    const emptyTitle = await app.mux.fetch("/api/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "   " }),
    });
    assert.strictEquals(emptyTitle.status, 400);

    const invalidPatch = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ done: "yes" }),
    });
    assert.strictEquals(invalidPatch.status, 400);

    const emptyPatch = await app.mux.fetch(`/api/tasks/${created.id}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    assert.strictEquals(emptyPatch.status, 400);

    // 405 + Allow on wrong method
    const wrongMethod = await app.mux.fetch("/api/tasks", {
      method: "PUT",
      headers: { Accept: "application/json" },
    });
    assert.strictEquals(wrongMethod.status, 405);
    assert.strictEquals(wrongMethod.header("allow"), "GET, HEAD, POST");
    assert.strictEquals(wrongMethod.json<{ status: number }>().status, 405);
  } finally {
    await db.close();
  }
});

test("Static CSS serving and path-traversal protection", async () => {
  const { db, app } = await setupApp();
  try {
    const cssRes = await app.mux.fetch("/assets/style.css");
    assert.strictEquals(cssRes.status, 200);
    assert.strictEquals(cssRes.header("content-type"), "text/css; charset=utf-8");
    assert.ok(cssRes.text().includes(".task-list"));

    for (const traversalPath of [
      "/assets/../app.ts",
      "/assets/%2e%2e/app.ts",
      "/assets/..%2fapp.ts",
    ]) {
      const badRes = await app.mux.fetch(traversalPath);
      assert.ok(
        badRes.status === 400 || badRes.status === 404,
        `expected 400 or 404 for ${traversalPath}, got ${badRes.status}`,
      );
      assert.ok(!badRes.text().includes("createApp"));
    }
  } finally {
    await db.close();
  }
});

test("Middleware attaches security headers, request ID, and structured logs", async () => {
  const { db, app, logs } = await setupApp();
  try {
    const generatedRes = await app.mux.fetch("/api/health");
    assert.strictEquals(generatedRes.status, 200);
    const rid = generatedRes.header("x-request-id");
    assert.ok(typeof rid === "string" && rid.length > 0);
    assert.strictEquals(generatedRes.header("x-content-type-options"), "nosniff");
    assert.strictEquals(generatedRes.header("x-frame-options"), "DENY");
    assert.strictEquals(generatedRes.header("referrer-policy"), "no-referrer");

    const propagatedRes = await app.mux.fetch("/api/health", {
      headers: { "X-Request-Id": "trace-42_ok" },
    });
    assert.strictEquals(propagatedRes.status, 200);
    assert.strictEquals(propagatedRes.header("x-request-id"), "trace-42_ok");

    assert.strictEquals(logs.length, 2);
    assert.strictEquals(logs[0].method, "GET");
    assert.strictEquals(logs[0].path, "/api/health");
    assert.strictEquals(logs[0].status, 200);
    assert.strictEquals(logs[0].requestId, rid);
    assert.strictEquals(logs[1].requestId, "trace-42_ok");
  } finally {
    await db.close();
  }
});

test("Live loopback server binds ephemeral port and shuts down cleanly", async () => {
  const { db, app } = await setupApp();
  let server: Server | undefined;
  try {
    server = serve({ host: "127.0.0.1", port: 0 }, app.mux);
    assert.strictEquals(server.host, "127.0.0.1");
    assert.ok(server.port > 0, `expected ephemeral port > 0, got ${server.port}`);
    assert.strictEquals(server.addr, `127.0.0.1:${server.port}`);
    assert.strictEquals(server.url, `http://127.0.0.1:${server.port}`);
  } finally {
    if (server) {
      await server.close();
      await server.finished;
    }
    await db.close();
  }
});
