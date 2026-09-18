import { useEffect, useRef } from "react";
import { CrepeBuilder } from "@milkdown/crepe/builder";
import { cursor } from "@milkdown/crepe/feature/cursor";
import { listItem } from "@milkdown/crepe/feature/list-item";
import { linkTooltip } from "@milkdown/crepe/feature/link-tooltip";
import { imageBlock } from "@milkdown/crepe/feature/image-block";
import { blockEdit } from "@milkdown/crepe/feature/block-edit";
import { placeholder } from "@milkdown/crepe/feature/placeholder";
import { toolbar } from "@milkdown/crepe/feature/toolbar";
import { codeMirror } from "@milkdown/crepe/feature/code-mirror";
import { table } from "@milkdown/crepe/feature/table";
import { oneDark } from "@codemirror/theme-one-dark";
import { editorViewCtx } from "@milkdown/kit/core";
import type { EditorView } from "@milkdown/kit/prose/view";
// Crepe's common/style.css minus latex (which pulls KaTeX's CSS and fonts), top-bar, diff and ai.
import "@milkdown/crepe/theme/common/prosemirror.css";
import "@milkdown/crepe/theme/common/reset.css";
import "@milkdown/crepe/theme/common/block-edit.css";
import "@milkdown/crepe/theme/common/code-mirror.css";
import "@milkdown/crepe/theme/common/cursor.css";
import "@milkdown/crepe/theme/common/image-block.css";
import "@milkdown/crepe/theme/common/link-tooltip.css";
import "@milkdown/crepe/theme/common/list-item.css";
import "@milkdown/crepe/theme/common/placeholder.css";
import "@milkdown/crepe/theme/common/toolbar.css";
import "@milkdown/crepe/theme/common/table.css";
import "@milkdown/crepe/theme/classic.css";
import "./milkdown-heroui-theme.css";
import { supportedLanguages, renderCodeBlockPreview } from "./codeBlockPreview";
import { githubAlerts } from "./githubAlerts";

interface RichTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

/** The mounted editor, module-level so callers (the find bar) need no ref plumbing. */
let liveCrepe: CrepeBuilder | null = null;

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
    // Crepe's LaTeX feature treats any "$...$" span as inline math, so prose like
    // "$12.9 billion ($13B value)" renders as italic KaTeX. Most markdown tools
    // don't do that; math still works in a ```latex fenced block (see codeBlockPreview).
    // The builder rather than `new Crepe` because Crepe's entry imports every
    // feature, and the LaTeX one drags ~450 KB of KaTeX in even when switched off.
    // Features are added in Crepe's default order, minus LaTeX.
    const crepe = new CrepeBuilder({ root: rootRef.current, defaultValue: initialValue })
      .addFeature(cursor)
      .addFeature(listItem)
      .addFeature(linkTooltip)
      .addFeature(imageBlock)
      .addFeature(blockEdit, {
        blockHandle: {
          getOffset: () => (window.innerWidth < 640 ? 4 : 12),
        },
      })
      .addFeature(placeholder)
      .addFeature(toolbar)
      .addFeature(codeMirror, {
        // Crepe's own default, which the builder does not apply.
        theme: oneDark,
        languages: supportedLanguages,
        renderPreview: renderCodeBlockPreview,
        previewOnlyByDefault: true,
        previewToggleText: (previewOnlyMode) => (previewOnlyMode ? "Edit" : "Preview"),
      })
      .addFeature(table);
    crepe.editor.use(githubAlerts);
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
