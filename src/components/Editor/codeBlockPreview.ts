import { LanguageDescription } from "@codemirror/language";
import { languages as defaultLanguages } from "@codemirror/language-data";
import { markdown } from "@codemirror/lang-markdown";

export const mermaidLanguage = LanguageDescription.of({
  name: "Mermaid",
  alias: ["mermaid", "flowchart", "diagram"],
  load: async () => markdown(),
});

export const supportedLanguages: LanguageDescription[] = [
  mermaidLanguage,
  ...defaultLanguages,
];

/**
 * Renderers for fenced blocks, loaded the first time one is actually on screen.
 * Mermaid is ~3.5 MB and KaTeX ~300 KB; `import()` puts each in its own chunk,
 * so a document with no diagrams and no math never downloads either.
 */
const PREVIEWS = [
  { languages: ["mermaid", "flowchart", "diagram"], load: () => import("./preview/mermaid") },
  { languages: ["latex", "math", "katex", "tex"], load: () => import("./preview/katex") },
];

export function renderCodeBlockPreview(
  language: string,
  content: string,
  applyPreview: (value: null | string | HTMLElement) => void,
): void | null | string | HTMLElement {
  const trimmed = content.trim();
  if (!trimmed) return null;

  const lang = language.trim().toLowerCase();
  const preview = PREVIEWS.find((p) => p.languages.includes(lang));
  if (!preview) return null;

  // Undefined, not a value: the panel is filled through applyPreview once the
  // chunk has loaded and the renderer has run.
  preview
    .load()
    .then((m) => m.render(trimmed))
    .then(applyPreview)
    .catch(() => applyPreview(null));
}
