# Proposal: ECMAScript npm modules for neojs

**Status:** request for design approval, not implemented. Based on neojs `f41970e`.
The constraints in **Scope** are the request; the policies below are recommendations, not
decisions already made. No Node.js or npm compatibility percentage is implied.

## Request and scope

Let a script import compatible ECMAScript packages from the npm registry without a separate
install command or a project manifest. Automatically download missing packages into a shared
per-user cache, normally `~/.cache/neojs/packages` on Linux. Like Deno, prefer an explicit
`npm:` namespace; Bun is another useful package-resolution comparison, not a compatibility
target. Examples below illustrate the requested interface, not current neojs behavior:

```js
import { value } from "npm:example-esm@1.2.3";
import { helper } from "npm:@example/tools@^2.0.0/helper";
```

Three independent capabilities must not be conflated:

1. **ESM npm packages:** resolution, download and execution of packages whose complete used
   graph fits neojs's supported ESM and host APIs. This is the first target.
2. **`node:*` APIs:** separately approved, opt-in compatibility modules, each with an explicit
   tested API subset and permission implications. Package loading does not enable them.
3. **Project `package.json`:** eventually consider using its dependency declarations and
   bare imports. Reading a downloaded package's own manifest for resolution is necessary now
   and does not mean project manifests, npm scripts, or a `node_modules` layout are supported.

**No CommonJS**, including transitive modules, wrappers, `require`, `createRequire`, or
automatic CJS-to-ESM conversion. No lifecycle scripts, native addons, npm CLI execution,
private registry credentials, project `node_modules`, or full Node globals are promised.
Failure to support these is an explicit limitation, not an instruction to execute a fallback.
JSR is separate and out of scope; npm packages are not JSR packages.

## Verified baseline and reference behavior

- The [neojs README](../README.md#limits) and `neojs-core/src/loader.rs` at the base commit
  document a filesystem ESM loader, erasable `.ts`/`.mts`, built-in bare and `neo:` names,
  rejected `npm:`/`node:`/`jsr:` specifiers, no JSON module imports, no permission system,
  and no sandbox. Existing `crypto`, `path`, `os`, etc. are neojs APIs, not automatically
  Node-compatible. The loader currently rejects `.cjs`; ordinary `.js` is executed as ESM,
  and unresolved CommonJS globals fail at runtime. Windows is unverified.
- [Deno vendor documentation](https://docs.deno.com/runtime/fundamentals/node/) describes
  `npm:` imports, automatic global caching, optional project manifests and separate Node APIs.
  It also supports CommonJS and some native addons; those are **not** adopted here.
- [Bun vendor module-resolution documentation](https://bun.com/docs/runtime/module-resolution)
  describes bare package imports, conditional exports and both module systems. Its extension
  search and CommonJS behavior are not a specification for neojs.
- [Node v26.13.1 package documentation](https://nodejs.org/docs/v26.13.1/api/packages.html)
  specifies package `type`, `exports`, `imports`, and condition ordering. `exports` takes
  precedence over `main` and encapsulates unexported paths; conditional keys are matched in
  manifest order. `.mjs` is ESM, `.cjs` CommonJS, and `.js` depends on the nearest package
  type; ambiguous cases have Node-specific behavior. Neojs need not claim full Node behavior.
- [npm CLI v11 package metadata](https://docs.npmjs.com/cli/v11/configuring-npm/package-json/)
  describes dependency ranges, peer/optional dependencies and platform metadata; its
  [lockfile documentation](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/)
  records exact trees, resolved artifacts and integrity. These are vendor documentation, not
  evidence that neojs currently implements them.

## Proposed resolution contract

1. Parse `npm:name@range/subpath` and `npm:@scope/name@range/subpath` unambiguously; accept
   exact versions, npm-compatible ranges and dist-tags, including omitted versions as a
   documented tag. Validate names and encode registry paths rather than concatenating paths.
   Initially use the public npm registry over HTTPS; arbitrary Git, file, link, URL and alias
   dependencies fail with an actionable unsupported-specifier error. Registry substitution
   and authenticated registries need a separate security design.
2. Fetch registry metadata and select a version using a reviewed npm-semver implementation;
   do not approximate range/prerelease semantics. Use registry tarball URL and integrity
   metadata. Resolve each package's declared runtime dependency edges recursively, allowing
   multiple versions and cycles. Resolve bare imports *inside* packages relative to the
   importing package's dependency graph, not a global latest version; reject undeclared
   dependencies rather than falling through to the registry. Artifact identity alone is not
   module identity: propagate a graph-instance identity including the resolved peer context
   through resolution, and key module instances by that identity and canonical module path.
   The same artifact used with different peer bindings must not share a module instance or
   resolve imports using the other context. Existing neojs built-in bare
   specifiers retain their behavior; define and test the collision rule inside packages before
   release. An explicit `npm:` import must never resolve to a neojs built-in.
3. Honor package `exports` for root, subpaths, patterns, null exclusions, nested conditions
   and target validation; honor internal `#` imports including mappings to declared external
   packages. Preserve manifest key order. Recommended active conditions: `import` and
   `default`; do not claim `node`, `node-addons`, `require`, `browser`, or `types`. Never
   bypass an `exports` rejection by falling back to `main` or a filesystem deep import.
   Without `exports`, specify and test a restricted `main`/root fallback separately; reject
   ambiguous or unsupported forms rather than guessing. Package self-reference and URL/path
   normalization need explicit tests.
4. Classify **every loaded package file**, including nested package scopes: accept `.mjs`,
   and `.js` whose nearest package manifest explicitly says `type: module`; reject `.cjs`,
   `.cts`, and `.js` under `type: commonjs`. Recommend rejecting package `.js` with no
   explicit type instead of guessing; `.mjs` remains valid even in a CommonJS package.
   This stricter package policy must not silently change existing local `.js` behavior.
   An ESM file that calls an unavailable `require` still fails when executed; absence of a
   CJS wrapper is not proof that arbitrary source is free of CommonJS assumptions.
5. Retain the existing erasable-TypeScript-only behavior for `.ts`/`.mts` if package TS is
   admitted; do not imply type checking, JSX, decorators, TS transforms, or declaration-file
   execution. JSON and Wasm imports remain unsupported until separately designed. Most
   real-world packages may fail because of missing APIs, globals, JSON, scripts or addons
   despite having ESM entrypoints; document the failing import and reason.

### Graph and reproducibility

Treat dependencies, optional dependencies and peers as distinct graph edges. Proposed first
release resolves ordinary transitive dependencies; supports optional edges only with documented
platform selection and explicit omission diagnostics; requires a compatible peer supplied in
the relevant parent context, errors on missing/conflicting required peers, and permits an
absent optional peer. Do not silently auto-install peers or treat them as ordinary children.
Reject unsupported bundled dependencies or shrinkwrap semantics until implemented. Check
`os`, `cpu`, and applicable `libc` restrictions; incompatible required packages fail, while
incompatible optional packages may be omitted. Development dependencies are not runtime edges.

Propose a versioned, project-owned lockfile recording roots, full graph including peer context
and omitted optional edges, exact package versions, registry identity, artifact URL and strong
integrity digest. A normal first run may resolve and write it; a frozen mode must reject a
missing, stale or incomplete lock without mutation or network metadata resolution. Offline
mode uses only verified cached artifacts and fails clearly on misses. An explicit update
re-resolves ranges/tags and produces a reviewable lock diff. Exact CLI names, lockfile path,
whether lock creation is automatic, and interoperability with npm lockfiles are open decisions;
never silently treat a partial npm lock as equivalent. Cache presence alone is not a lockfile.

## Cache, transport and security

- Default Linux location is `~/.cache/neojs/packages`; honor an explicit cache override and
  define XDG handling. On Windows use an appropriate per-user local cache directory (proposed
  `%LOCALAPPDATA%\\neojs\\packages`), not a literal Unix path. Test drive letters, UNC paths,
  case collisions, reserved names and long paths; do not require administrator rights.
- Partition metadata by registry origin and packages by immutable identity and verified digest.
  A cache hit verifies the executed extracted bytes against a trusted record derived from the
  verified archive, or re-extracts them from the reverified archive before execution. Verifying
  the tarball alone does not detect changes to previously extracted files. Protect that record
  from untrusted cache modification, or rederive it from the verified archive; ensure verification
  and execution do not race with replacement of the files. Detect corruption and refetch only when
  policy permits. Strong integrity verifies bytes against the expected digest, not publisher
  identity or benign behavior. Reject missing/unsupported integrity in the proposed first
  release rather than execute unverified artifacts; determine compatibility impact.
- Unlocked tags/ranges need explicit metadata freshness/conditional revalidation policy:
  propose online revalidation each run, offline only with a lock, and no silent stale fallback.
  Locked runs must not change versions as tags move. Bound network redirects, time, artifact
  sizes, decompressed sizes, file counts and dependency graph work. Require HTTPS and validate
  redirect destinations; private/authenticated registries remain out of scope.
- Extract to a private temporary location; reject absolute paths, traversal, unsafe links,
  device files and platform-specific aliasing. Prevent link-following and time-of-check races;
  validate final containment with OS-appropriate primitives. Publish verified complete entries
  atomically with interprocess coordination; interrupted downloads never appear valid.
  Test simultaneous processes, stale locks, crashes, corruption and cleanup. Avoid sharing
  writable cache state across users; same-user compromise is not excluded.
- Fetching on import is network access initiated by the host loader, distinct from JavaScript
  `fetch`. Make that clear to the user and provide offline/frozen controls. The current runtime
  has no permission system: package code shares available process capabilities and is **not
  sandboxed**. A future permission model must separately cover loader network/cache operations
  and any enabled Node filesystem, network, environment, subprocess or FFI APIs.
- Do not run `preinstall`, `install`, `postinstall`, `prepare` or other lifecycle scripts;
  report ignored scripts and explain that packages depending on generated files may fail.
  Reject native `.node` addons and do not build or download bindings via scripts. Review
  dependency licences, provenance and vulnerability policy; integrity and HTTPS alone do not
  establish trust. Consider minimum release age and signature verification separately.

## Phases and acceptance criteria

**0. Contract and fixtures.** Approve grammar, conditions, fallback, peers, cache directory,
lockfile and network policy; build a local mock registry with deterministic package tarballs.
No real registry access is required in tests.

**1. ESM npm loading.** From an empty isolated cache, import exact and ranged unscoped and
scoped fixtures, root and subpath, with transitive dependencies, cycles, and two versions of
one package. A fixture uses the same package artifact under two graph instances with different
peer versions: each resolves its own peer and receives a distinct module instance. A second
run reuses verified artifacts; offline locked run succeeds. Export
conditions honor key order, `#` mappings work, and private/unexported or escaping targets fail.
`.cjs`, CommonJS-typed `.js`, ambiguous `.js` under the proposed policy, unsupported formats,
unknown built-ins and unsupported specs fail with package, referrer and reason. Existing
local imports and neojs built-ins remain unchanged.

**2. Reproducibility and hardening, required before stable release.** Semver, prerelease and
tag fixtures, lock update/frozen/offline behavior, peer conflicts and optional/platform cases
are deterministic. Integrity mismatch, corrupted cache, unsafe archives, interrupted writes,
concurrent processes, redirect abuse and resource limits fail closed. Test on Linux and
Windows; document any unverified platform. Modify an extracted `.js` file before a cache hit:
the modified bytes must never execute, including in offline mode; verification fails or
re-extraction restores verified bytes. Confirm no lifecycle script or native addon runs.

**3. Independent extensions.** Specify and test selected opt-in `node:*` APIs, then consider
project manifest and `node_modules` integration, TypeScript improvements and JSR as separate
proposals. Each needs compatibility and security review; none unlocks CommonJS implicitly.

## Decisions needed before implementation

- Which collision rule applies when a package imports a bare name already used by a neojs
  built-in? How will users explicitly address each without breaking existing scripts?
- Which `main` fallback and unsupported package metadata forms are in the first slice?
  How will `npm:` roots supply peers without a project manifest?
- Is automatic project lockfile creation acceptable, and what is the CLI/embedding API for
  frozen, offline, update and cache override modes? What happens in read-only directories?
- Which integrity algorithms and registry artifact origins are permitted, and what limits,
  cache-eviction policy and release-age policy are appropriate?
- Which Node API, if any, is first to opt in, and what exact permissions and tests are needed?
  Is package TS admitted immediately or deferred?

The source documentation does not establish how many real packages satisfy this subset, nor
whether a particular Rust resolver safely implements it. Measure against a named fixture corpus
and audit candidate libraries before promising coverage. Prototype: none; this is a design
request and the examples above are illustrative, not executable proof.
