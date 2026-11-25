import { createHmac, randomBytes } from "vael:crypto";
import type { Database } from "vael:db";
import {
  badRequest,
  createRouter,
  html,
  HttpError,
  json,
  type LogEntry,
  logger,
  noContent,
  notFound,
  recover,
  redirect,
  requestId,
  securityHeaders,
  type ServeMux,
  setCookie,
} from "vael:http";
import {
  createTask,
  deleteTask,
  getTask,
  listTasks,
  type TaskFilter,
  type TaskPatch,
  toggleTask,
  updateTask,
} from "./db.ts";
import { createAppTemplates, renderHomePage } from "./templates.ts";

/** Dependencies injected into `createApp` using plain Go-style parameters. */
export interface AppOptions {
  db: Database;
  staticDir: string;
  logSink?: (entry: LogEntry) => void;
  cookieSecret?: Uint8Array | string;
}

/** Application container returned by `createApp`. */
export interface WebApp {
  mux: ServeMux;
}

const FLASH_COOKIE_NAME = "flash";
const MAX_TITLE_LENGTH = 200;
const MAX_FLASH_PREVIEW_CHARS = 80;

function formatFlashPreview(prefix: string, title: string): string {
  const chars = Array.from(title);
  const preview = chars.length > MAX_FLASH_PREVIEW_CHARS
    ? `${chars.slice(0, MAX_FLASH_PREVIEW_CHARS - 1).join("")}…`
    : title;
  return `${prefix}: ${preview}`;
}

function constantTimeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

function signFlashCookie(message: string, secret: Uint8Array | string): string {
  const encoded = encodeURIComponent(message);
  const mac = createHmac("sha256", secret).update(encoded).digest("hex");
  return `${encoded}.${mac}`;
}

function verifyFlashCookie(
  rawValue: string | undefined,
  secret: Uint8Array | string,
): string {
  if (!rawValue) {
    return "";
  }
  const dot = rawValue.lastIndexOf(".");
  if (dot <= 0) {
    return "";
  }
  const encoded = rawValue.slice(0, dot);
  const mac = rawValue.slice(dot + 1);
  const expected = createHmac("sha256", secret).update(encoded).digest("hex");
  if (!constantTimeEqual(mac, expected)) {
    return "";
  }
  try {
    return decodeURIComponent(encoded);
  } catch {
    return "";
  }
}

function parseTaskFilter(raw: string | null): TaskFilter {
  const value = (raw ?? "all").trim().toLowerCase();
  if (value === "" || value === "all") {
    return "all";
  }
  if (value === "open" || value === "done") {
    return value;
  }
  throw badRequest(
    "invalid status filter; expected all, open, or done",
    "INVALID_STATUS",
  );
}

function assertNonEmptyTitle(raw: unknown): string {
  if (typeof raw !== "string") {
    throw badRequest("task title must be a string", "INVALID_TITLE");
  }
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    throw badRequest("task title must not be empty", "INVALID_TITLE");
  }
  if (Array.from(trimmed).length > MAX_TITLE_LENGTH) {
    throw badRequest(
      `task title must be at most ${MAX_TITLE_LENGTH} characters`,
      "TITLE_TOO_LONG",
    );
  }
  return trimmed;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/**
 * Construct the frameworkless Go-style `ServeMux` application with explicit
 * dependency injection for the database, static assets directory, and logger.
 */
export function createApp({
  db,
  staticDir,
  logSink,
  cookieSecret = randomBytes(32),
}: AppOptions): WebApp {
  const templates = createAppTemplates();
  const mux = createRouter();

  mux.use(
    recover(),
    requestId(),
    securityHeaders(),
    logger({ sink: logSink }),
  );

  mux.static("/assets", staticDir);

  mux.get("/", async (req) => {
    const filter = parseTaskFilter(req.query.get("status"));
    const tasks = await listTasks(db, filter);
    const rawFlash = req.cookies[FLASH_COOKIE_NAME];
    const flash = verifyFlashCookie(rawFlash, cookieSecret);

    let res = html(
      renderHomePage(templates, {
        heading: "vaeljs task board",
        filter,
        tasks,
        totalCount: tasks.length,
        flash,
      }),
    );

    if (rawFlash !== undefined) {
      res = setCookie(res, {
        name: FLASH_COOKIE_NAME,
        value: "",
        path: "/",
        maxAge: 0,
        httpOnly: true,
        sameSite: "Lax",
      });
    }
    return res;
  });

  mux.post("/tasks", async (req) => {
    const form = await req.form();
    const title = assertNonEmptyTitle(form.get("title") ?? "");
    const task = await createTask(db, title);
    const res = redirect("/", 303);
    return setCookie(res, {
      name: FLASH_COOKIE_NAME,
      value: signFlashCookie(
        formatFlashPreview("Created task", task.title),
        cookieSecret,
      ),
      path: "/",
      httpOnly: true,
      sameSite: "Lax",
    });
  });

  mux.post("/tasks/{id}/toggle", async (req) => {
    const id = req.param("id") ?? "";
    const updated = await toggleTask(db, id);
    if (!updated) {
      throw notFound("task not found", "TASK_NOT_FOUND");
    }
    const label = updated.done ? "Completed" : "Reopened";
    return setCookie(redirect("/", 303), {
      name: FLASH_COOKIE_NAME,
      value: signFlashCookie(
        formatFlashPreview(label, updated.title),
        cookieSecret,
      ),
      path: "/",
      httpOnly: true,
      sameSite: "Lax",
    });
  });

  mux.post("/tasks/{id}/delete", async (req) => {
    const id = req.param("id") ?? "";
    const deleted = await deleteTask(db, id);
    if (!deleted) {
      throw notFound("task not found", "TASK_NOT_FOUND");
    }
    return setCookie(redirect("/", 303), {
      name: FLASH_COOKIE_NAME,
      value: signFlashCookie("Task deleted", cookieSecret),
      path: "/",
      httpOnly: true,
      sameSite: "Lax",
    });
  });

  mux.group("/api", (api) => {
    // Ensure REST API errors always serialize as JSON regardless of Accept header.
    api.use((next) => async (req, ctx) => {
      try {
        return await next(req, ctx);
      } catch (err) {
        if (err instanceof HttpError) {
          const payload: Record<string, unknown> = {
            error: err.message,
            status: err.status,
          };
          if (err.code !== undefined) {
            payload.code = err.code;
          }
          return json(payload, { status: err.status, headers: err.headers });
        }
        throw err;
      }
    });

    api.get("/health", () => json({ ok: true, runtime: "vaeljs" }));

    api.get("/tasks", async (req) => {
      const filter = parseTaskFilter(req.query.get("status"));
      const tasks = await listTasks(db, filter);
      return json({ tasks, count: tasks.length });
    });

    api.post("/tasks", async (req) => {
      const body = await req.json<unknown>();
      if (!isPlainObject(body)) {
        throw badRequest("request body must be a JSON object", "INVALID_BODY");
      }
      const title = assertNonEmptyTitle(body.title);
      const task = await createTask(db, title);
      return json(task, { status: 201 });
    });

    api.get("/tasks/{id}", async (req) => {
      const id = req.param("id") ?? "";
      const task = await getTask(db, id);
      if (!task) {
        throw notFound("task not found", "TASK_NOT_FOUND");
      }
      return json(task);
    });

    api.patch("/tasks/{id}", async (req) => {
      const id = req.param("id") ?? "";
      const body = await req.json<unknown>();
      if (!isPlainObject(body)) {
        throw badRequest("request body must be a JSON object", "INVALID_BODY");
      }
      const patch: TaskPatch = {};
      if ("title" in body && body.title !== undefined) {
        patch.title = assertNonEmptyTitle(body.title);
      }
      if ("done" in body && body.done !== undefined) {
        if (typeof body.done !== "boolean") {
          throw badRequest("done must be a boolean", "INVALID_DONE");
        }
        patch.done = body.done;
      }
      if (patch.title === undefined && patch.done === undefined) {
        throw badRequest("patch must include title or done", "EMPTY_PATCH");
      }
      const updated = await updateTask(db, id, patch);
      if (!updated) {
        throw notFound("task not found", "TASK_NOT_FOUND");
      }
      return json(updated);
    });

    api.delete("/tasks/{id}", async (req) => {
      const id = req.param("id") ?? "";
      const deleted = await deleteTask(db, id);
      if (!deleted) {
        throw notFound("task not found", "TASK_NOT_FOUND");
      }
      return noContent();
    });
  });

  return { mux };
}
