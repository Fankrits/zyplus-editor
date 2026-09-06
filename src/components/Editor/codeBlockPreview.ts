import { LanguageDescription } from "@codemirror/language";
import { languages as defaultLanguages } from "@codemirror/language-data";
import { markdown } from "@codemirror/lang-markdown";
import { extensionManager } from "../../extensions/extensionManager";

export const mermaidLanguage = LanguageDescription.of({
  name: "Mermaid",
  alias: ["mermaid", "flowchart", "diagram"],
  load: async () => markdown(),
});

export const supportedLanguages: LanguageDescription[] = [
  mermaidLanguage,
  ...defaultLanguages,
];

export function renderCodeBlockPreview(
  language: string,
  content: string,
  applyPreview: (value: null | string | HTMLElement) => void,
): void | null | string | HTMLElement {
  const trimmed = content.trim();
  if (!trimmed) return null;

  const renderer = extensionManager.getPreviewRenderer(language);
  if (renderer) {
    return renderer(language, trimmed, applyPreview);
  }

  return null;
}

