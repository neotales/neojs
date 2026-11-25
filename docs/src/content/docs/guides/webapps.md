---
title: Go-Style WebApps
description: Build frameworkless TypeScript web applications with vael:http ServeMux, vael:db SQLite transactions, vael:template SSR, and zero-socket mux.fetch() tests.
---

VaelJS lets you build complete server-rendered and JSON web applications with **zero third-party frameworks**, combining `vael:http`, `vael:db`, `vael:template`, `vael:uuid`, and `vael:crypto`.

The repository includes a full runnable task-board prototype under [`examples/webapp/`](https://github.com/neotales/vaeljs/tree/dev/examples/webapp):

```sh
# Start the server (defaults: HOST=127.0.0.1, PORT=8080, DB_PATH=:memory:)
vjs examples/webapp/main.ts

# Persist to a SQLite file on port 3000
PORT=3000 DB_PATH=./tasks.sqlite vjs examples/webapp/main.ts

# Run zero-socket mux.fetch() and loopback server tests
vjs test examples/webapp/app.test.ts
```

## Architecture: Go-Style Without Boilerplate

- **Plain dependency injection:** Wire `createApp({ db, staticDir, logSink })` with ordinary TypeScript objects instead of decorators, containers, or global singletons.
- **Immutable request/response:** Handlers have signature `(req: HttpRequest, ctx: RequestContext) => HttpResponse | Promise<HttpResponse>`, avoiding `ResponseWriter` partial-write and header-after-body bugs.
- **Thrown `HttpError`s:** Handlers throw `badRequest(...)` or `notFound(...)`, and `ServeMux` + `recover()` convert them to HTTP responses while masking unexpected exceptions as `500`.

## 1. Routing & Middleware (`vael:http`)

`createRouter()` builds a Go 1.22+-style `ServeMux` supporting path parameters (`{id}`), route groups, composable middleware, and traversal-safe static file serving:

```ts
import {
  badRequest,
  createRouter,
  html,
  HttpError,
  json,
  logger,
  notFound,
  recover,
  redirect,
  requestId,
  securityHeaders,
  setCookie,
} from "vael:http";

export function createApp({ db, staticDir, logSink }) {
  const mux = createRouter();

  mux.use(recover(), requestId(), securityHeaders(), logger({ sink: logSink }));
  mux.static("/assets", staticDir);

  mux.get("/", async (req) => {
    const status = req.query.get("status") ?? "all";
    const tasks = await listTasks(db, status);
    return html(renderHomePage({ tasks, status }));
  });

  mux.post("/tasks/{id}/toggle", async (req) => {
    const id = req.param("id") ?? "";
    const updated = await toggleTask(db, id);
    if (!updated) {
      throw notFound("task not found", "TASK_NOT_FOUND");
    }
    return setCookie(redirect("/", 303), {
      name: "flash",
      value: "Task updated",
      path: "/",
      httpOnly: true,
      sameSite: "Lax",
    });
  });

  mux.group("/api", (api) => {
    api.get("/health", () => json({ ok: true, runtime: "vaeljs" }));
    api.post("/tasks", async (req) => {
      const body = await req.json<{ title?: unknown }>();
      if (typeof body?.title !== "string" || body.title.trim() === "") {
        throw badRequest("task title must not be empty", "INVALID_TITLE");
      }
      const task = await createTask(db, body.title.trim());
      return json(task, { status: 201 });
    });
  });

  return { mux };
}
```

## 2. SQLite & Atomic Transactions (`vael:db`)

`vael:db` supports both positional (`?`) and named (`@name`) parameter bindings, bounded row results (`RowLimit::default()`: 1,000 rows, 1 MiB), and atomic transactions:

```ts
import { type Database, openSqlite } from "vael:db";
import { v7 } from "vael:uuid";

export async function openTaskDb(database = ":memory:"): Promise<Database> {
  const db = await openSqlite(database);
  await db.execute(`
    CREATE TABLE IF NOT EXISTS tasks (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      done INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL
    )
  `);
  return db;
}

export async function createTask(db: Database, title: string) {
  const id = v7();
  const createdAt = new Date().toISOString();
  await db.execute(
    "INSERT INTO tasks (id, title, done, created_at) VALUES (@id, @title, @done, @created_at)",
    { id, title, done: 0, created_at: createdAt },
  );
  return { id, title, done: false, createdAt };
}

export async function toggleTask(db: Database, id: string) {
  return db.transaction(async (tx) => {
    const row = await tx.queryOne(
      "SELECT id, title, done, created_at FROM tasks WHERE id = ?",
      [id],
    );
    if (!row) return null;
    const nextDone = !Boolean(Number(row.done));
    await tx.execute("UPDATE tasks SET done = @done WHERE id = @id", {
      id,
      done: nextDone ? 1 : 0,
    });
    return { ...row, done: nextDone };
  });
}
```

:::note[Always Use `tx` Inside `db.transaction`]
`:memory:` SQLite pools default to `max_connections: 1`. Inside `db.transaction(async (tx) => ...)`, issue every query against `tx`—never the outer `db` handle.
:::

## 3. XSS-Safe Layouts & Partials (`vael:template`)

Create a reusable `TemplateSet` with `htmlEscape: true` and Sprig helpers (`default`, `trim`, `title`, `upper`):

```ts
import { createTemplateSet } from "vael:template";

const templates = createTemplateSet({ sprig: true, htmlEscape: true })
  .add(
    "layout",
    `{{define "layout"}}<!doctype html>
<html>
<head><title>{{block "title" .}}VaelJS Tasks{{end}}</title></head>
<body><main>{{block "content" .}}{{end}}</main></body>
</html>{{end}}`,
  )
  .add(
    "home",
    `{{define "title"}}{{.heading | default "tasks" | trim | title}}{{end}}
{{define "content"}}
  <h1>{{.heading}}</h1>
  <ul>{{range .tasks}}<li>{{.title | trim}}</li>{{end}}</ul>
{{end}}
{{template "layout" .}}`,
  );

const body = templates.render("home", {
  heading: "vaeljs task board",
  tasks: [{ title: "<script>alert(1)</script>" }],
});
// Interpolated `<script>` is automatically escaped to `&lt;script&gt;...`
```

## 4. Startup & Zero-Socket `mux.fetch()` Testing

Start the live `hyper`/`tokio` HTTP/1.1 server in `main.ts`:

```ts
import { serve } from "vael:http";

const server = serve({ host: "127.0.0.1", port: 8080 }, app.mux);
console.log(`Listening on ${server.url}`);
await server.finished;
```

In tests, call `app.mux.fetch(...)` directly in memory—running your router, middleware, templates, cookies, and SQLite queries with zero open TCP sockets:

```ts
import assert from "vael:assert";
import test from "vael:test";

test("POST /api/tasks creates a task and escapes XSS on SSR", async () => {
  const db = await openTaskDb(":memory:");
  try {
    const app = createApp({ db, staticDir: "./public" });
    const res = await app.mux.fetch("/api/tasks", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "<script>alert(1)</script>" }),
    });
    assert.strictEquals(res.status, 201);

    const home = await app.mux.fetch("/");
    assert.strictEquals(home.status, 200);
    assert.ok(!home.text().includes("<script>"));
  } finally {
    await db.close();
  }
});
```
