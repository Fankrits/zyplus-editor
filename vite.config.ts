import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import pkg from "./package.json";

import fs from "node:fs";
import path from "node:path";

import type { Plugin } from "vite";

const host = process.env.TAURI_DEV_HOST;

const localExtensionsPlugin: Plugin = {
  name: "serve-local-extensions",
  configureServer(server) {
    const base = path.resolve(process.cwd(), "extensions/dist");
    server.middlewares.use("/extensions-dist", (req, res, next) => {
      const cleanUrl = decodeURIComponent((req.url || "").split("?")[0]);
      // Resolved and checked, so `/extensions-dist/../../anything` serves nothing.
      const filePath = path.resolve(base, "." + cleanUrl);
      if (!filePath.startsWith(base + path.sep)) return next();
      const stat = fs.statSync(filePath, { throwIfNoEntry: false });
      if (stat?.isFile()) {
        const ext = path.extname(filePath);
        const contentType =
          ext === ".js"
            ? "application/javascript"
            : ext === ".css"
            ? "text/css"
            : "text/plain";
        res.setHeader("Content-Type", contentType);
        return fs.createReadStream(filePath).pipe(res);
      }
      next();
    });
  },
};

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), localExtensionsPlugin],

  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },

  // The Tauri webview is modern (WebKit / WebView2), so skip the downleveling
  // Vite's default target does — notably async/await into generator machines.
  build: {
    target: "esnext",
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
