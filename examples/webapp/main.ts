import { serve } from "neo:http";
import { dirname, join } from "neo:path";
import { env } from "node:process";
import { fileURLToPath } from "node:url";
import { createApp } from "./app.ts";
import { openTaskDb } from "./db.ts";

function parsePort(raw: string | undefined, fallback: number): number {
  if (!raw || raw.trim() === "") {
    return fallback;
  }
  const parsed = Number(raw);
  if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
    throw new RangeError(`Invalid PORT "${raw}": expected integer 0..=65535`);
  }
  return parsed;
}

const host = env.HOST && env.HOST.trim() !== "" ? env.HOST.trim() : "127.0.0.1";
const port = parsePort(env.PORT, 8080);
const dbPath = env.DB_PATH && env.DB_PATH.trim() !== ""
  ? env.DB_PATH.trim()
  : ":memory:";

const here = dirname(fileURLToPath(import.meta.url));
const staticDir = join(here, "public");

const db = await openTaskDb(dbPath);
try {
  const app = createApp({ db, staticDir });
  const server = serve({ host, port }, app.mux);
  console.log(`Listening on ${server.url}`);
  await server.finished;
} finally {
  await db.close();
}
