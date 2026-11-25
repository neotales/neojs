import { type Database, openSqlite } from "neo:db";
import { v7 } from "neo:uuid";

/** Supported task status filters for listing tasks. */
export type TaskFilter = "all" | "open" | "done";

/** Domain model for a persisted task row. */
export interface Task {
  id: string;
  title: string;
  done: boolean;
  createdAt: string;
}

/** Fields accepted when updating an existing task. */
export interface TaskPatch {
  title?: string;
  done?: boolean;
}

const CREATE_TASKS_TABLE_SQL = `
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
)`;

function rowToTask(row: Record<string, unknown>): Task {
  return {
    id: String(row.id),
    title: String(row.title),
    done: Boolean(Number(row.done)),
    createdAt: String(row.created_at),
  };
}

/**
 * Ensure the `tasks` table exists on `db`.
 */
export async function initSchema(db: Database): Promise<void> {
  await db.execute(CREATE_TASKS_TABLE_SQL);
}

/**
 * Open a SQLite database (defaulting to `":memory:"`) and initialize its schema.
 */
export async function openTaskDb(database = ":memory:"): Promise<Database> {
  const db = await openSqlite(database);
  try {
    await initSchema(db);
    return db;
  } catch (err) {
    await db.close();
    throw err;
  }
}

/**
 * List tasks filtered by completion status, newest first.
 * Uses positional (`?`) parameters when filtering by status.
 */
export async function listTasks(
  db: Database,
  filter: TaskFilter = "all",
): Promise<Task[]> {
  if (filter === "open" || filter === "done") {
    const rows = await db.query(
      "SELECT id, title, done, created_at FROM tasks WHERE done = ? ORDER BY created_at DESC, id DESC",
      [filter === "done" ? 1 : 0],
    );
    return rows.map(rowToTask);
  }
  const rows = await db.query(
    "SELECT id, title, done, created_at FROM tasks ORDER BY created_at DESC, id DESC",
  );
  return rows.map(rowToTask);
}

/**
 * Fetch a single task by UUIDv7 `id`, or `null` if not found.
 * Uses positional (`?`) parameter binding.
 */
export async function getTask(db: Database, id: string): Promise<Task | null> {
  const row = await db.queryOne(
    "SELECT id, title, done, created_at FROM tasks WHERE id = ?",
    [id],
  );
  return row ? rowToTask(row) : null;
}

/**
 * Create a new task with a time-ordered UUIDv7 `id`.
 * Uses named (`@id`, `@title`, `@done`, `@created_at`) parameter binding.
 */
export async function createTask(db: Database, title: string): Promise<Task> {
  const id = v7();
  const createdAt = new Date().toISOString();
  await db.execute(
    "INSERT INTO tasks (id, title, done, created_at) VALUES (@id, @title, @done, @created_at)",
    {
      id,
      title,
      done: 0,
      created_at: createdAt,
    },
  );
  return {
    id,
    title,
    done: false,
    createdAt,
  };
}

/**
 * Atomically update an existing task's `title` and/or `done` flag inside a transaction.
 * Returns the updated task, or `null` if no task matched `id`.
 */
export async function updateTask(
  db: Database,
  id: string,
  patch: TaskPatch,
): Promise<Task | null> {
  return db.transaction(async (tx) => {
    const existing = await getTask(tx, id);
    if (!existing) {
      return null;
    }
    const nextTitle = patch.title !== undefined ? patch.title : existing.title;
    const nextDone = patch.done !== undefined ? patch.done : existing.done;
    await tx.execute(
      "UPDATE tasks SET title = @title, done = @done WHERE id = @id",
      {
        id,
        title: nextTitle,
        done: nextDone ? 1 : 0,
      },
    );
    return {
      ...existing,
      title: nextTitle,
      done: nextDone,
    };
  });
}

/**
 * Atomically flip a task's `done` flag inside a transaction.
 * Returns the updated task, or `null` if no task matched `id`.
 */
export async function toggleTask(db: Database, id: string): Promise<Task | null> {
  return db.transaction(async (tx) => {
    const existing = await getTask(tx, id);
    if (!existing) {
      return null;
    }
    const nextDone = !existing.done;
    await tx.execute("UPDATE tasks SET done = @done WHERE id = @id", {
      id,
      done: nextDone ? 1 : 0,
    });
    return {
      ...existing,
      done: nextDone,
    };
  });
}

/**
 * Delete a task by `id` using positional (`?`) parameter binding.
 * Returns `true` if a row was deleted.
 */
export async function deleteTask(db: Database, id: string): Promise<boolean> {
  const result = await db.execute("DELETE FROM tasks WHERE id = ?", [id]);
  return result.rowsAffected > 0;
}
