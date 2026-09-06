import { useEffect, useRef } from "react";
import { Crepe } from "@milkdown/crepe";
import { editorViewCtx } from "@milkdown/kit/core";
import type { EditorView } from "@milkdown/kit/prose/view";
import "@milkdown/crepe/theme/common/style.css";
import "@milkdown/crepe/theme/classic.css";
import "./milkdown-heroui-theme.css";
import { supportedLanguages, renderCodeBlockPreview } from "./codeBlockPreview";

interface RichTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

/** The mounted editor, module-level so callers (the find bar) need no ref plumbing. */
let liveCrepe: Crepe | null = null;

/** The rich editor's ProseMirror view, or null before it finishes mounting. */
export function findRichEditor(): EditorView | null {
  try {
    return liveCrepe?.editor.action((ctx) => ctx.get(editorViewCtx)) ?? null;
  } catch {
    return null;
  }
}

export function RichTextEditor({ initialValue, onChange }: RichTextEditorProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!rootRef.current) return;
    const crepe = new Crepe({
      root: rootRef.current,
      defaultValue: initialValue,
      // Crepe's LaTeX feature treats any "$...$" span as inline math, so prose like
      // "$12.9 billion ($13B value)" renders as italic KaTeX. Most markdown tools
      // don't do that; math still works in a ```latex fenced block (see codeBlockPreview).
      features: { [Crepe.Feature.Latex]: false },
      featureConfigs: {
        [Crepe.Feature.BlockEdit]: {
          blockHandle: {
            getOffset: () => (window.innerWidth < 640 ? 4 : 12),
          },
        },
        [Crepe.Feature.CodeMirror]: {
          languages: supportedLanguages,
          renderPreview: renderCodeBlockPreview,
          previewOnlyByDefault: true,
          previewToggleText: (previewOnlyMode) => (previewOnlyMode ? "Edit" : "Preview"),
        },
      },
    });
    crepe.on((listener) => {
      listener.markdownUpdated((_ctx, markdown, prevMarkdown) => {
        if (markdown !== prevMarkdown) onChangeRef.current(markdown);
      });
    });
    crepe.create();
    liveCrepe = crepe;
    return () => {
      if (liveCrepe === crepe) liveCrepe = null;
      crepe.destroy();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={rootRef} className="h-full overflow-y-auto px-1 sm:px-6 py-2 sm:py-6" />;
}
