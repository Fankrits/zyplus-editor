import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import pkg from "./package.json";

import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";

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

// Tauri embeds every file Vite writes to dist/ in the binary, and the startup path is
// what every launch parses. Two rules keep both small, and fail the build rather than
// let either grow quietly:
//  - The extension libraries are built by `build:extensions` and downloaded on demand,
//    so the app itself must never bundle them. (The finished bundles the web build ships
//    above are plain assets, not chunks, so they don't count.)
//  - The editors are lazy chunks. A helper shared with the eager UI that a vendor chunk
//    happens to capture is enough to pull one onto the startup path.
const EXTENSION_ONLY_LIBS =
  /[\\/]node_modules[\\/](?:mermaid|katex|chart\.js|tabulator-tables|papaparse|vanilla-jsoneditor|lossless-json)[\\/]/;
// Brotli KB of JavaScript the app loads before it can draw anything (desktop measured 184;
// see docs/performance.md). Raise it on purpose, not by accident: `bun run analyze` shows
// what joined the startup path.
const STARTUP_BUDGET_KB = 200;
const EDITOR_ONLY_LIBS =
  /[\\/]node_modules[\\/](?:@milkdown|prosemirror-[^\\/]+|@codemirror[\\/](?:view|state))[\\/]/;

const guardBundle: Plugin = {
  name: "guard-bundle",
  generateBundle(_, bundle) {
    const chunks = new Map(
      Object.values(bundle).flatMap((file) => (file.type === "chunk" ? [[file.fileName, file] as const] : [])),
    );

    for (const chunk of chunks.values()) {
      const leaked = chunk.moduleIds.find((id) => EXTENSION_ONLY_LIBS.test(id));
      if (leaked) {
        this.error(
          `${leaked} is bundled into ${chunk.fileName}. Extension libraries load on demand ` +
            `from src/extensions/bundles; the app must not import them.`,
        );
      }
    }

    const startup = new Set<string>();
    const visit = (name: string) => {
      if (startup.has(name)) return;
      startup.add(name);
      chunks.get(name)?.imports.forEach(visit);
    };
    for (const chunk of chunks.values()) if (chunk.isEntry) visit(chunk.fileName);
    const startupKB =
      [...startup].reduce(
        (bytes, name) =>
          bytes +
          zlib.brotliCompressSync(chunks.get(name)!.code, {
            params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 },
          }).length,
        0,
      ) / 1024;
    if (startupKB > STARTUP_BUDGET_KB) {
      this.error(
        `The startup path is ${startupKB.toFixed(0)} KB of brotli JS, over its ${STARTUP_BUDGET_KB} KB budget ` +
          `(STARTUP_BUDGET_KB in vite.config.ts). Run \`bun run analyze\` to see what joined it.`,
      );
    }
    for (const name of startup) {
      const eager = chunks.get(name)?.moduleIds.find((id) => EDITOR_ONLY_LIBS.test(id));
      if (eager) {
        this.error(
          `${eager} is in ${name}, which loads at startup. The editors are lazy chunks; ` +
            `check what the entry imports from it (or from a vendor group in vite.config.ts).`,
        );
      }
    }
  },
};

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => ({
  plugins: [
    react(),
    tailwindcss(),
    localExtensionsPlugin,
    bundleExtensionsPlugin,
    guardBundle,
    // `bun run analyze`. Written next to the project, not into dist/, which the binary embeds.
    // Brotli is the size that counts: Tauri embeds its assets brotli-compressed.
    mode === "analyze" &&
      (await import("rollup-plugin-visualizer")).visualizer({
        filename: "bundle-report.html",
        gzipSize: true,
        brotliSize: true,
      }),
  ],

  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },

  build: {
    // The Tauri webview is modern (WebKit / WebView2), so skip the downleveling
    // Vite's default target does — notably async/await into generator machines.
    target: "esnext",
    // Whatever lands in dist/ is embedded in the binary, maps included.
    sourcemap: false,
    rolldownOptions: {
      // Vendor debug logging (Lezer's parser trace, HeroUI's logger) is dead weight in a
      // release. Only log and debug are droppable: the app reports its own failures through
      // warn and error, which a release build's devtools would otherwise never show.
      // `debugger` statements already go with minification, and `build.minify` stays
      // Vite's to decide, so `vite build --minify false` still gives a readable bundle.
      treeshake: { manualPureFunctions: ["console.log", "console.debug"] },
      output: {
        // Named vendor chunks. The editors are lazy, so each group has to hold code that
        // loads together: CodeMirror inside the Milkdown group would make the plain-text
        // editor fetch all of Milkdown, and either inside the React group would put them
        // on the startup path. clsx is here because the eager UI uses it too. The
        // per-language CodeMirror packages stay out on purpose: one lazy chunk each.
        codeSplitting: {
          groups: [
            { name: "react-vendor", test: /[\\/]node_modules[\\/](?:react|react-dom|scheduler|clsx)[\\/]/ },
            {
              name: "codemirror",
              test: /[\\/]node_modules[\\/](?:@codemirror[\\/](?:state|view|language|commands|search|autocomplete|lint|theme-one-dark)|@lezer[\\/](?:common|lr|highlight)|style-mod|w3c-keyname|crelt|@marijn)[\\/]/,
              priority: 1,
            },
            {
              name: "editor-core",
              test: /[\\/]node_modules[\\/](?:@milkdown|@vue|prosemirror-[^\\/]+|orderedmap|rope-sequence|remark-[^\\/]+|micromark[^\\/]*|mdast-[^\\/]+|unified|unist-[^\\/]+|vfile[^\\/]*)[\\/]/,
            },
          ],
        },
      },
    },
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
