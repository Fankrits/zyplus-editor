import type { ExtensionManifest } from "./types";

const isDev = Boolean(typeof import.meta !== "undefined" && import.meta.env?.DEV);

/**
 * Release builds fetch the bundles straight out of the repository, so this has
 * to name a ref that exists. It used to say `extension` — a branch deleted
 * after it merged — which made every install in every shipped build a 404, and
 * with it Mermaid and KaTeX. Renaming or deleting the ref breaks installs for
 * users already running an older build, so it tracks the default branch.
 */
export const ASSET_REF = "main";

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
    cssUrl: getAssetUrl("katex.css"),
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
