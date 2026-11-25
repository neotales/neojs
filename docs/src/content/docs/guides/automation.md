---
title: Automation Recipes
description: Practical automation scripts using vael:fs, node:child_process, vael:ssh, vael:age, vael:template, vael:cron, and shared vael:test suites.
---

VaelJS is built for deterministic, shell-free operational scripting. Most automation modules (`vael:fs`, `vael:ssh`, `vael:template`, `vael:crypto`, `node:child_process`, `vael:test`) work identically on both **`vjs`** and **`vjs-lite`**, while async scheduling (`vael:cron`) and binary encryption (`vael:age`) target **`vjs`**.

## Atomic Filesystem Operations (`vael:fs`)

Use `writeFileAtomic` to replace configuration or state files via a sibling `0o600` temporary file and `sync_all` + atomic rename, and use `maxBytes` on `readTextFile` to bound reads:

```ts
import { exists, mkdir, readDir, readTextFile, writeFileAtomic } from "vael:fs";
import { join } from "vael:path";
import { Buffer } from "node:buffer";

const outDir = "./build-state";
if (!exists(outDir)) {
  mkdir(outDir, { recursive: true });
}

const configPath = join(outDir, "manifest.json");
const payload = Buffer.from(JSON.stringify({ updatedAt: new Date().toISOString() }, null, 2), "utf8");
writeFileAtomic(configPath, payload, { sync: true, overwrite: true });

const raw = readTextFile(configPath, { maxBytes: 1024 * 1024 });
console.log("Wrote:", raw);

for (const entry of readDir(outDir)) {
  console.log(entry.name, entry.isFile ? "file" : "other");
}
```

`vael:fs` is synchronous and runs on both `vjs` and `vjs-lite`.

## Shell-Free Subprocesses (`node:child_process`)

Run external tools without spawning `/bin/sh` or `cmd.exe` by passing an explicit argument array to `execFileSync` or `spawnSync`:

```ts
import { execFileSync, spawnSync } from "node:child_process";

const status = execFileSync("git", ["status", "--short"], {
  encoding: "utf8",
  timeout: 5000,
});
console.log(status.trim() || "working tree clean");

const check = spawnSync("git", ["rev-parse", "--abbrev-ref", "HEAD"], {
  encoding: "utf8",
});
if (check.status !== 0) {
  throw new Error(`git failed (${check.status}): ${check.stderr}`);
}
console.log("Branch:", check.stdout.trim());
```

Because no shell is invoked, arguments containing spaces or metacharacters are never re-parsed.

## Remote SSH Execution (`vael:ssh`)

`vael:ssh` connects synchronously with **mandatory host-key verification** (`hostKey` or `knownHosts`) and never reads `~/.ssh/config` or `~/.ssh/known_hosts`:

```ts
import { connect } from "vael:ssh";
import { readFile } from "vael:fs";
import { env } from "node:process";

const conn = connect({
  host: "app-01.internal",
  port: 22,
  username: "deploy",
  knownHosts: "app-01.internal ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...",
  privateKey: readFile(env.DEPLOY_KEY_PATH ?? "/run/secrets/deploy_ed25519"),
  connectTimeoutMs: 10_000,
  execTimeoutMs: 30_000,
});

try {
  const res = conn.exec("uptime");
  if (res.status !== 0) {
    throw new Error(`Remote command exited ${res.status}`);
  }
  console.log(new TextDecoder().decode(res.stdout).trim());
} finally {
  conn.close();
}
```

Auth requires exactly one of `password`, `privateKey` (in-memory PKCS#8 or OpenSSH key bytes), or `agent: true` (Unix `SSH_AUTH_SOCK`). Output is bounded to `MAX_OUTPUT_BYTES` (`8 MiB`), and remote `stderr` is discarded.

## Age-Encrypted Secrets (`vael:age`, `vjs` only)

Encrypt and decrypt binary age v1 payloads (up to 8 MiB plaintext / 9 MiB ciphertext) using X25519 keys:

```ts
import { decrypt, encrypt, generateIdentity } from "vael:age";
import { Buffer } from "node:buffer";

const { identity, recipient } = generateIdentity();
const secret = Buffer.from("db_password=super-secret\n", "utf8");

const ciphertext = encrypt(recipient, secret);
const recovered = decrypt(identity, ciphertext);
console.log(Buffer.from(recovered).toString("utf8").trim());
```

## Config Rendering (`vael:template`)

Render Go `text/template` files with Sprig helpers (`default`, `upper`, `trim`, `title`, and more). Disable `htmlEscape` for non-HTML configs such as nginx, systemd, or YAML:

```ts
import { render } from "vael:template";

const unit = render(
  `[Service]
ExecStart=/usr/local/bin/{{.app | trim}} --port {{.port | default 8080}}
Environment=LOG_LEVEL={{.logLevel | default "info" | upper}}
`,
  { app: "worker", logLevel: "warn" },
  { sprig: true, htmlEscape: false },
);

console.log(unit);
```

## UTC Cron Scheduling (`vael:cron`, `vjs` only)

Schedule in-process UTC five-field jobs with overlap protection (at most one callback runs at a time; undisposed jobs keep the event loop alive):

```ts
import { cron, next } from "vael:cron";

const upcoming = next("*/5 * * * *");
console.log("Next UTC boundary:", upcoming?.toISOString());

const job = cron(
  "*/5 * * * *",
  async () => {
    console.log("Running periodic health check at", new Date().toISOString());
  },
  {
    onEvent: (event) => {
      if (event.type === "error") {
        console.error("Cron callback error:", event.error);
      }
    },
  },
);

// Call job.dispose() when shutting down cleanly
```

## Shared Cross-Engine Tests (`vael:test` + `vael:assert`)

Write tests using `vael:test` and `vael:assert` so the same suite passes under both `vjs test` and `vjs-lite test`:

```ts
import assert from "vael:assert";
import test from "vael:test";
import { render } from "vael:template";

test("template renders default port", () => {
  const out = render("port={{.port | default 8080}}", {}, { htmlEscape: false });
  assert.strictEquals(out, "port=8080");
});
```

Avoid `test(name, fn, { timeout })` in shared suites because `vjs-lite` has no host timers and rejects `timeout` with `VAEL_TEST_NO_TIMER`.
