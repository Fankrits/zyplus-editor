import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import pkg from "./package.json";

import fs from "node:fs";
import path from "node:path";

import type { Plugin } from "vite";

const host = process.env.TAURI_DEV_HOST;

const EXTENSIONS_DIST = path.resolve(process.cwd(), "extensions/dist");

const localExtensionsPlugin: Plugin = {
  name: "serve-local-extensions",
  configureServer(server) {
    server.middlewares.use("/extensions-dist", (req, res, next) => {
      let cleanUrl: string;
      try {
        cleanUrl = decodeURIComponent((req.url || "").split("?")[0]);
      } catch {
        return next(); // a malformed %-escape is a 404, not a crash
      }
      // Resolved and checked, so `/extensions-dist/../../anything` serves nothing.
      const filePath = path.resolve(EXTENSIONS_DIST, "." + cleanUrl);
      if (!filePath.startsWith(EXTENSIONS_DIST + path.sep)) return next();
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
        return fs
          .createReadStream(filePath)
          .on("error", () => res.destroy())
          .pipe(res);
      }
      next();
    });
  },
};

/**
 * Ships `extensions/dist` inside the web build, at `/extensions-dist/`, so the
 * browser installs extensions from the same deploy it is running (see
 * `assetUrl` in src/extensions/catalog.ts). The desktop build fetches them from
 * its release tag instead, and skips this to keep the installer small.
 */
const bundleExtensionsPlugin: Plugin = {
  name: "bundle-extensions",
  apply: "build",
  generateBundle() {
    if (process.env.TAURI_ENV_PLATFORM) return;
    if (!fs.existsSync(EXTENSIONS_DIST)) {
      this.error("extensions/dist is missing; run `bun run build:extensions` before building the web app.");
    }
    for (const name of fs.readdirSync(EXTENSIONS_DIST)) {
      this.emitFile({
        type: "asset",
        fileName: `extensions-dist/${name}`,
        source: fs.readFileSync(path.join(EXTENSIONS_DIST, name)),
      });
    }
  },
};

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), localExtensionsPlugin, bundleExtensionsPlugin],

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
