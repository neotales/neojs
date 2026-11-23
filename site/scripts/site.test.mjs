// Checks the generated site, not the content source. Run after `pnpm build`.
// A missing build is a failure rather than a skip: a green test run that
// checked nothing is the failure mode this file exists to avoid.

import assert from "node:assert/strict";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parse } from "node-html-parser";

const siteRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(siteRoot, "dist");

const builtInModules = [
  "age",
  "assert",
  "chars",
  "console",
  "cron",
  "crypto",
  "os",
  "path",
  "slices",
  "ssh",
  "stdio",
  "strings",
  "test",
  "uuid",
];

const expectedPages = [
  "",
  "getting-started/",
  "cli/",
  "modules/",
  "modules/age/",
  "modules/assert/",
  "modules/chars/",
  "modules/console/",
  "modules/cron/",
  "modules/crypto/",
  "modules/os/",
  "modules/path/",
  "modules/slices/",
  "modules/ssh/",
  "modules/stdio/",
  "modules/strings/",
  "modules/test/",
  "modules/uuid/",
  "runtime/",
  "lite/",
  "v8-vs-lite/",
  "remote-security/",
  "limits/",
  "db-api/",
  "contributing/",
];

async function exists(target) {
  try {
    await stat(target);
    return true;
  } catch {
    return false;
  }
}

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await walk(target)));
    else files.push(target);
  }
  return files;
}

function pageFile(route) {
  if (route === "") return path.join(dist, "index.html");
  return path.join(dist, route, "index.html");
}

function plainDecode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function sidebarHrefs(html) {
  const root = parse(html);
  const sidebar = root.querySelector('nav.sidebar[aria-label="Main"]');
  assert.ok(sidebar, "the rendered Starlight sidebar nav is missing");

  return new Set(
    sidebar
      .querySelectorAll("a[href]")
      .map((link) => link.getAttribute("href"))
      .filter((href) => typeof href === "string"),
  );
}

async function resolveInternalHref(href) {
  if (!href.startsWith("/") || href.startsWith("//")) return true;
  const withoutHash = href.split("#")[0].split("?")[0];
  if (withoutHash === "") return true;
  const decoded = plainDecode(withoutHash);
  const relative = decoded.replace(/^\/+/, "");

  if (withoutHash.endsWith("/")) {
    return exists(path.join(dist, relative, "index.html"));
  }

  const direct = path.join(dist, relative);
  if (await exists(direct)) return true;
  const asPage = path.join(dist, relative, "index.html");
  return exists(asPage);
}

test("documented built-in imports use the neo: prefix", async () => {
  const docs = path.join(siteRoot, "src", "content", "docs");
  const files = (await walk(docs)).filter(
    (file) => file.endsWith(".md") || file.endsWith(".mdx"),
  );
  const bare = [];

  for (const file of files) {
    const text = await readFile(file, "utf8");
    for (const name of builtInModules) {
      const bareImport = new RegExp(`from\\s+"${name}"|import\\(\\s*"${name}"\\s*\\)`);
      if (bareImport.test(text)) bare.push(`${path.relative(siteRoot, file)}: ${name}`);
    }
  }

  assert.deepEqual(bare, [], `bare built-in imports are not canonical:\n${bare.join("\n")}`);
});

test("the production build exists", async () => {
  assert.equal(
    await exists(path.join(dist, "index.html")),
    true,
    "dist/index.html is missing; run `pnpm build` before `pnpm test`",
  );
});

test("Pagefind search output is generated", async () => {
  const pagefind = path.join(dist, "pagefind");
  assert.equal(await exists(path.join(pagefind, "pagefind.js")), true, "pagefind.js is missing");
  assert.equal(
    await exists(path.join(pagefind, "pagefind-entry.json")),
    true,
    "pagefind-entry.json is missing",
  );

  const indexDir = path.join(pagefind, "index");
  assert.equal(await exists(indexDir), true, "the Pagefind index directory is missing");
  const indexFiles = await readdir(indexDir);
  assert.ok(indexFiles.length > 0, "the Pagefind index directory is empty");

  const entry = JSON.parse(await readFile(path.join(pagefind, "pagefind-entry.json"), "utf8"));
  const pageCount = entry.languages?.en?.page_count ?? 0;
  assert.ok(pageCount > 0, "the Pagefind index contains no pages");
  assert.ok(
    pageCount >= expectedPages.length,
    `the Pagefind index has ${pageCount} pages, expected at least ${expectedPages.length}`,
  );

  const home = await readFile(path.join(dist, "index.html"), "utf8");
  assert.match(home, /pagefind|Search/i, "the built home page has no search control");
});

test("every expected page is built", async () => {
  for (const route of expectedPages) {
    assert.equal(await exists(pageFile(route)), true, `missing page: /${route}`);
  }
});

test("the rendered sidebar links every expected page", async () => {
  // A regular documentation page carries Starlight's full sidebar. The splash
  // home page deliberately does not, so check a content page. Parse only
  // <nav class="sidebar" aria-label="Main">, so a content link cannot mask a
  // missing navigation entry.
  const html = await readFile(path.join(dist, "getting-started", "index.html"), "utf8");
  const links = sidebarHrefs(html);

  for (const route of expectedPages) {
    const href = `/${route}`;
    assert.ok(links.has(href), `the rendered sidebar does not link to ${href}`);
  }
});

test("internal links resolve inside dist", async () => {
  const htmlFiles = (await walk(dist)).filter((file) => file.endsWith(".html"));
  const broken = [];

  for (const file of htmlFiles) {
    const html = await readFile(file, "utf8");
    for (const match of html.matchAll(/\shref="([^"]+)"/g)) {
      const href = match[1];
      if (!(await resolveInternalHref(href))) {
        broken.push(`${path.relative(dist, file)} -> ${href}`);
      }
    }
  }

  assert.deepEqual(broken, [], `broken internal links:\n${broken.join("\n")}`);
});
