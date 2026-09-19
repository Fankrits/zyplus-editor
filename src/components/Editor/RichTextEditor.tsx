import { useEffect, useRef, useState } from "react";
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
import { extensionManager } from "../../extensions/extensionManager";
import { DiagramViewer } from "./DiagramViewer";

const ZOOM_BUTTON_CLASS = "diagram-zoom-button";
const FULL_SCREEN_ICON = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" aria-hidden="true"><path d="M15.5 21c1.4 0 2.1 0 2.66-.17a3.6 3.6 0 0 0 2.67-2.67c.17-.56.17-1.26.17-2.66M21 8.5c0-1.4 0-2.1-.17-2.66a3.6 3.6 0 0 0-2.67-2.67C17.6 3 16.9 3 15.5 3M8.5 21c-1.4 0-2.1 0-2.66-.17a3.6 3.6 0 0 1-2.67-2.67C3 17.6 3 16.9 3 15.5M3 8.5c0-1.4 0-2.1.17-2.66a3.6 3.6 0 0 1 2.67-2.67C6.4 3 7.1 3 8.5 3"/></svg>`;

/**
 * Milkdown's code block toolbar is a Vue component with no slot for extra
 * buttons, so the expand button is added next to "Edit" on every block whose
 * preview is a rendered diagram, and dropped again when the preview goes.
 */
function syncZoomButtons(root: HTMLElement) {
  for (const block of root.querySelectorAll<HTMLElement>(".milkdown-code-block")) {
    const group = block.querySelector(".tools-button-group");
    if (!group) continue;
    const hasDiagram = !!block.querySelector(".preview > svg");
    const button = group.querySelector(`.${ZOOM_BUTTON_CLASS}`);
    if (hasDiagram && !button) {
      const el = document.createElement("button");
      el.type = "button";
      el.className = ZOOM_BUTTON_CLASS;
      el.title = "View full screen";
      el.setAttribute("aria-label", "View diagram full screen");
      el.innerHTML = FULL_SCREEN_ICON;
      group.append(el);
    } else if (!hasDiagram && button) {
      button.remove();
    }
  }
}

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
  const [zoomedDiagram, setZoomedDiagram] = useState<SVGSVGElement | null>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    syncZoomButtons(root);
    const observer = new MutationObserver(() => syncZoomButtons(root));
    observer.observe(root, { childList: true, subtree: true });
    const onClick = (e: MouseEvent) => {
      const button = (e.target as Element).closest(`.${ZOOM_BUTTON_CLASS}`);
      const svg = button?.closest(".milkdown-code-block")?.querySelector<SVGSVGElement>(".preview > svg");
      if (!svg) return;
      e.preventDefault();
      e.stopPropagation();
      setZoomedDiagram(svg);
    };
    root.addEventListener("click", onClick, true);
    return () => {
      observer.disconnect();
      root.removeEventListener("click", onClick, true);
    };
  }, []);

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
    // Milkdown only asks for a preview when a block's text or language changes, so
    // blocks already on screen would keep their old preview until edited. A fresh
    // code_block node view makes ProseMirror redraw them, which asks again.
    let enabled = extensionManager.getEnabledIds().join();
    const unsubscribe = extensionManager.subscribe(() => {
      const next = extensionManager.getEnabledIds().join();
      if (next === enabled) return;
      enabled = next;
      const view = findRichEditor();
      const codeBlock = view?.props.nodeViews?.code_block;
      if (!view || !codeBlock) return;
      // A new function identity is what tells ProseMirror to rebuild the views.
      view.setProps({ nodeViews: { ...view.props.nodeViews, code_block: (...args) => codeBlock(...args) } });
    });
    return () => {
      unsubscribe();
      if (liveCrepe === crepe) liveCrepe = null;
      crepe.destroy();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <>
      <div ref={rootRef} className="h-full overflow-y-auto px-1 sm:px-6 py-2 sm:py-6" />
      {zoomedDiagram && <DiagramViewer svg={zoomedDiagram} onClose={() => setZoomedDiagram(null)} />}
    </>
  );
}
