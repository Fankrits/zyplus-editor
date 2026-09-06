import katex from "katex";
import type { ExtensionRuntime } from "../types";

export default function createKatexExtension(): ExtensionRuntime {
  return {
    id: "katex",
    renderCodeBlockPreview(
      _language: string,
      content: string,
      applyPreview: (value: null | string | HTMLElement) => void,
    ) {
      const trimmed = content.trim();
      if (!trimmed) return null;

      try {
        const rendered = katex.renderToString(trimmed, {
          throwOnError: false,
          displayMode: true,
        });
        applyPreview(rendered);
      } catch {
        applyPreview(null);
      }

      return undefined;
    },
  };
}
