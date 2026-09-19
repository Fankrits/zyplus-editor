import type { ExtensionManifest } from "./types";
import checksums from "./checksums.json";

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

function getAssetUrl(filename: string): string {
  if (isDev) {
    return `/extensions-dist/${filename}`;
  }
  return `https://raw.githubusercontent.com/Fankrits/zyplus-editor/${ASSET_REF}/extensions/dist/${filename}`;
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
