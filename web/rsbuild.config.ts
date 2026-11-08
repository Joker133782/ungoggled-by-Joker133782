import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";
import tailwind from "@tailwindcss/postcss";
export default defineConfig({
  plugins: [pluginReact()],
  html: {
    title: "ungoggled",
    favicon: "./public/favicon.svg",
    meta: { "theme-color": "#0c0f14" },
    tags: [
      { tag: "link", attrs: { rel: "manifest", href: "/manifest.json" } },
      {
        tag: "link",
        attrs: { rel: "apple-touch-icon", href: "/icons/icon-192.png" },
      },
    ],
  },
  server: { host: "127.0.0.1", proxy: { "/api": "http://127.0.0.1:8080" } },
  tools: { postcss: { postcssOptions: { plugins: [tailwind()] } } },
});
