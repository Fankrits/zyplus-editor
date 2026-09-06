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
    server.middlewares.use("/extensions-dist", (req, res, next) => {
      const cleanUrl = (req.url || "").split("?")[0];
      const filePath = path.join(process.cwd(), "extensions/dist", cleanUrl);
      if (fs.existsSync(filePath) && fs.statSync(filePath).isFile()) {
        const ext = path.extname(filePath);
        const contentType =
          ext === ".js"
            ? "application/javascript"
            : ext === ".css"
            ? "text/css"
            : "text/plain";
        res.setHeader("Content-Type", contentType);
        res.setHeader("Access-Control-Allow-Origin", "*");
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
