// @ts-check
import { defineConfig } from "astro/config";
import mdx from "@astrojs/mdx";
import starlight from "@astrojs/starlight";
import tailwindcss from "@tailwindcss/vite";

// Starlight adds @astrojs/sitemap automatically, but a sitemap needs a public
// canonical `site` URL. This site has no assigned URL yet, so register a no-op
// integration with that name to keep the build clean. Replace it with the real
// @astrojs/sitemap integration and a `site` value once the URL is decided.
const unconfiguredSitemap = {
  name: "@astrojs/sitemap",
  hooks: {},
};

// The site is a standalone frontend. It has no dependency on the Rust
// workspace and does not change product behaviour.
export default defineConfig({
  integrations: [
    unconfiguredSitemap,
    starlight({
      title: "NeoJS",
      disable404Route: true,
      description:
        "Documentation for NeoJS: ECMAScript modules and erasable TypeScript on V8, plus the embeddable QuickJS NeoJS Lite runtime.",
      favicon: "/favicon.svg",
      logo: {
        src: "./public/favicon.svg",
        alt: "NeoJS",
      },
      customCss: ["./src/styles/global.css"],
      lastUpdated: true,
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/neotales/neojs",
        },
      ],
      sidebar: [
        {
          label: "Start here",
          items: [
            { label: "Overview", link: "/" },
            { label: "Getting started", slug: "getting-started" },
            { label: "Command line", slug: "cli" },
            { label: "Built-in modules", slug: "modules" },
          ],
        },
        {
          label: "Built-in modules",
          items: [
            { label: "age", slug: "modules/age" },
            { label: "assert", slug: "modules/assert" },
            { label: "chars", slug: "modules/chars" },
            { label: "console", slug: "modules/console" },
            { label: "crypto", slug: "modules/crypto" },
            { label: "os", slug: "modules/os" },
            { label: "path", slug: "modules/path" },
            { label: "slices", slug: "modules/slices" },
            { label: "ssh", slug: "modules/ssh" },
            { label: "stdio", slug: "modules/stdio" },
            { label: "strings", slug: "modules/strings" },
            { label: "test", slug: "modules/test" },
            { label: "uuid", slug: "modules/uuid" },
          ],
        },
        {
          label: "Runtimes",
          items: [
            { label: "V8 runtime (neojs-core)", slug: "runtime" },
            { label: "NeoJS Lite (QuickJS)", slug: "lite" },
            { label: "V8 and Lite compared", slug: "v8-vs-lite" },
          ],
        },
        {
          label: "Guides",
          items: [
            { label: "Remote loading and security", slug: "remote-security" },
            { label: "Limits and unsupported features", slug: "limits" },
          ],
        },
        {
          label: "Database API",
          items: [{ label: "neojs-dbapi", slug: "db-api" }],
        },
        {
          label: "Project",
          items: [{ label: "Building and contributing", slug: "contributing" }],
        },
      ],
    }),
    mdx(),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
