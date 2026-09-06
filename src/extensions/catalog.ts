import type { ExtensionManifest } from "./types";

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
    downloadUrl:
      "https://raw.githubusercontent.com/Fankrits/zyplus-editor/extension/extensions/dist/mermaid.js",
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
    downloadUrl:
      "https://raw.githubusercontent.com/Fankrits/zyplus-editor/extension/extensions/dist/katex.js",
    cssUrl:
      "https://raw.githubusercontent.com/Fankrits/zyplus-editor/extension/extensions/dist/katex.css",
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
