import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://neotales.github.io",
  integrations: [
    starlight({
      title: "VaelJS",
      description:
        "Fast, secure JavaScript & TypeScript runtimes for automation, webapps, and embedding — vjs (V8) and vjs-lite (QuickJS).",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/neotales/vaeljs",
        },
      ],
      customCss: ["./src/styles/custom.css"],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Installation", slug: "getting-started/installation" },
            { label: "Quickstart", slug: "getting-started/quickstart" },
            { label: "vjs vs vjs-lite", slug: "getting-started/vjs-vs-vjs-lite" },
          ],
        },
        {
          label: "CLI & Configuration",
          items: [
            { label: "Commands & Flags", slug: "cli/commands" },
            { label: "vael.jsonc & Workspaces", slug: "cli/vael-jsonc" },
            { label: "npm & Remote Loading", slug: "cli/npm-and-remote" },
          ],
        },
        {
          label: "Guides & Examples",
          items: [
            { label: "Automation Recipes", slug: "guides/automation" },
            { label: "Go-Style WebApps", slug: "guides/webapps" },
            { label: "Embedding vaeljs-lite", slug: "guides/embedding-vaeljs-lite" },
          ],
        },
        {
          label: "Module Reference",
          items: [
            { label: "Overview", slug: "modules/overview" },
            { label: "vael:http", slug: "modules/vael-http" },
            { label: "vael:db", slug: "modules/vael-db" },
            { label: "vael:template", slug: "modules/vael-template" },
            { label: "vael:fs", slug: "modules/vael-fs" },
            { label: "vael:ssh", slug: "modules/vael-ssh" },
            { label: "vael:crypto", slug: "modules/vael-crypto" },
            { label: "vael:age", slug: "modules/vael-age" },
            { label: "vael:os & vael:path", slug: "modules/vael-os-path" },
            { label: "vael:chars, strings & slices", slug: "modules/vael-text" },
            { label: "vael:test & vael:assert", slug: "modules/vael-testing" },
            { label: "vael:cron, console, stdio & uuid", slug: "modules/vael-utilities" },
            { label: "node:* Compatibility", slug: "modules/node-compat" },
          ],
        },
      ],
    }),
  ],
});
