import type { ExtensionManifest } from "./types";
import checksums from "./checksums.json";
import { isNativeApp } from "../lib/platform";

const isDev = Boolean(typeof import.meta !== "undefined" && import.meta.env?.DEV);

/**
 * Release builds fetch the bundles straight out of the repository, at the tag
 * the build was released from: a tag never moves, so what a build downloads is
 * exactly what it was released with, and `checksums.json` (written by
 * `build:extensions` at that same commit) proves it. Builds made before this
 * track `main`, which is why `main` has to keep serving `extensions/dist`.
 */
export const ASSET_REF =
  typeof __APP_VERSION__ !== "undefined" ? `v${__APP_VERSION__}` : "main";

/**
 * Where a bundle file is fetched from.
 *
 * The web build serves the bundles itself, from `/extensions-dist/` (emitted by
 * the `bundle-extensions` plugin in `vite.config.ts`). It cannot use the tag URL
 * below: its `package.json` version is never bumped outside a release run, so
 * the tag it named had no `extensions/dist` and every install was a 404. Serving
 * them from the same deploy also means the files always match `checksums.json`.
 *
 * The desktop build fetches from the tag it was released from instead, so the
 * installer does not carry them.
 */
export function assetUrl(filename: string, native: boolean, dev = isDev): string {
  if (dev || !native) return `/extensions-dist/${filename}`;
  return `https://raw.githubusercontent.com/Fankrits/zyplus-editor/${ASSET_REF}/extensions/dist/${filename}`;
}

function getAssetUrl(filename: string): string {
  return assetUrl(filename, isNativeApp);
}

export const EXTENSION_CATALOG: ExtensionManifest[] = [
  {
    id: "mermaid",
    name: "Mermaid Diagrams",
    description:
      "Renders rich interactive diagrams, sequence charts, git graphs, and flowcharts directly in markdown preview.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 3_670_000,
    languages: ["mermaid", "flowchart", "diagram"],
    downloadUrl: getAssetUrl("mermaid.js"),
    sha256: checksums["mermaid.js"],
  },
  {
    id: "katex",
    name: "LaTeX Math (KaTeX)",
    description:
      "Fast math typesetting engine that transforms LaTeX equations into formatted mathematical expressions.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 1_520_000,
    languages: ["latex", "math", "katex", "tex"],
    downloadUrl: getAssetUrl("katex.js"),
    sha256: checksums["katex.js"],
    cssUrl: getAssetUrl("katex.css"),
    cssSha256: checksums["katex.css"],
  },
  {
    id: "json",
    name: "JSON Editor",
    description:
      "Opens .json files in a tree, table or text view. Edit values, format, sort, search and repair, with big numbers kept exact.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 1_100_000,
    languages: [],
    fileExtensions: [".json"],
    fileModes: [
      { id: "text", label: "Text" },
      { id: "tree", label: "Tree" },
      { id: "table", label: "Table" },
    ],
    downloadUrl: getAssetUrl("json.js"),
    sha256: checksums["json.js"],
    cssUrl: getAssetUrl("json.css"),
    cssSha256: checksums["json.css"],
  },
  {
    id: "csv",
    name: "CSV / TSV Editor",
    description:
      "Opens .csv and .tsv files in a table or text view. Edit rows and columns with lossless round-tripping for ragged rows.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 900_000,
    languages: ["csv", "tsv"],
    fileExtensions: [".csv", ".tsv"],
    fileModes: [
      { id: "table", label: "Table" },
      { id: "text", label: "Text" },
    ],
    downloadUrl: getAssetUrl("csv.js"),
    sha256: checksums["csv.js"],
    cssUrl: getAssetUrl("csv.css"),
    cssSha256: checksums["csv.css"],
  },
  {
    id: "chart",
    name: "Charts",
    description: "Renders bar, line, pie, doughnut, and scatter charts from JSON configuration.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 210_000,
    languages: ["chart", "chartjs"],
    downloadUrl: getAssetUrl("chart.js"),
    sha256: checksums["chart.js"],
  },
  {
    id: "codefiles",
    name: "Code & Config Files",
    description: "Syntax highlighting and basic editor for various code, config and script files.",
    version: "1.0.0",
    author: "Zyplus",
    sizeBytesEstimate: 950_000,
    languages: [],
    fileExtensions: [
      ".yaml", ".yml", ".toml", ".xml", ".sh", ".bash", ".zsh", ".ini", ".env",
      ".css", ".html", ".js", ".jsx", ".ts", ".tsx", ".py", ".rs", ".go", ".sql",
      ".rb", ".php", ".c", ".cpp", ".h", ".java", ".kt", ".swift", ".dockerfile", ".gitignore",
    ],
    downloadUrl: getAssetUrl("codefiles.js"),
    sha256: checksums["codefiles.js"],
    cssUrl: getAssetUrl("codefiles.css"),
    cssSha256: checksums["codefiles.css"],
  },
];

export function getManifestById(id: string): ExtensionManifest | undefined {
  return EXTENSION_CATALOG.find((ext) => ext.id === id);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(1)} MB`;
}
