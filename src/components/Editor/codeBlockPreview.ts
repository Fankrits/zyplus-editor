import katex from "katex";
import mermaid from "mermaid";
import { LanguageDescription } from "@codemirror/language";
import { languages as defaultLanguages } from "@codemirror/language-data";
import { markdown } from "@codemirror/lang-markdown";

// Ensure CSSRuleList is iterable in WebKit / Safari environments (e.g. macOS Tauri webview)
if (typeof CSSRuleList !== "undefined" && !CSSRuleList.prototype[Symbol.iterator]) {
  CSSRuleList.prototype[Symbol.iterator] = Array.prototype[Symbol.iterator];
}

// Fallback for headless or mock environments where SVG getBBox is not implemented
if (typeof SVGElement !== "undefined" && !SVGElement.prototype.getBBox) {
  SVGElement.prototype.getBBox = function () {
    return { x: 0, y: 0, width: 100, height: 40 };
  };
}

export const mermaidLanguage = LanguageDescription.of({
  name: "Mermaid",
  alias: ["mermaid", "flowchart", "diagram"],
  load: async () => markdown(),
});

export const supportedLanguages: LanguageDescription[] = [
  mermaidLanguage,
  ...defaultLanguages,
];

const DIAGRAM_LANGUAGES = new Set(["mermaid", "flowchart", "diagram"]);
const MATH_LANGUAGES = new Set(["latex", "math", "katex", "tex"]);

export function renderCodeBlockPreview(
  language: string,
  content: string,
  applyPreview: (value: null | string | HTMLElement) => void,
): void | null | string | HTMLElement {
  const trimmed = content.trim();
  if (!trimmed) return null;

  const lang = language.trim().toLowerCase();

  if (MATH_LANGUAGES.has(lang)) {
    try {
      return katex.renderToString(trimmed, {
        throwOnError: false,
        displayMode: true,
      });
    } catch {
      return null;
    }
  }

  if (DIAGRAM_LANGUAGES.has(lang)) {
    const isDark =
      typeof document !== "undefined" &&
      (document.documentElement.classList.contains("dark") ||
        (typeof window !== "undefined" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches));

    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "loose",
      suppressErrorRendering: true,
      theme: isDark ? "dark" : "default",
      fontFamily: "inherit",
    });

    const renderId = `mermaid-${Math.random().toString(36).slice(2, 9)}`;
    mermaid
      .render(renderId, trimmed)
      .then(({ svg }) => {
        applyPreview(svg);
      })
      .catch(() => {
        applyPreview(null);
      });

    return undefined;
  }

  return null;
}
