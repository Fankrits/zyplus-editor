import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { useWorkspaceActions, useWorkspaceTabs } from "../../state/workspaceStore";
import { Wordmark } from "../Logo";
import { FIND_EVENT } from "../../lib/commands";

/**
 * The editors are the heaviest thing the app owns — Milkdown/ProseMirror for the
 * rich one, CodeMirror for the plain one, and between them about two thirds of
 * what used to be downloaded and parsed before anything could be painted. None of
 * it is needed to draw the window, the sidebar or the file tree, and a session
 * with no file open never needs it at all, so each one arrives on demand.
 *
 * The find bar is here too because it reaches into whichever editor is mounted:
 * importing it eagerly would drag both of them back in and undo the split.
 */
const RichTextEditor = lazy(() =>
  import("./RichTextEditor").then((m) => ({ default: m.RichTextEditor })),
);
const PlainTextEditor = lazy(() =>
  import("./PlainTextEditor").then((m) => ({ default: m.PlainTextEditor })),
);
const SearchBar = lazy(() => import("./SearchBar").then((m) => ({ default: m.SearchBar })));

export function EditorPane() {
  const { activeTab } = useWorkspaceTabs();
  const { dispatch } = useWorkspaceActions();
  const contentRef = useRef<HTMLDivElement>(null);
  const [search, setSearch] = useState<{ replace: boolean } | null>(null);

  useEffect(() => {
    const open = (e: Event) => {
      setSearch({ replace: (e as CustomEvent<{ replace?: boolean }>).detail?.replace ?? false });
    };
    window.addEventListener(FIND_EVENT, open);
    return () => window.removeEventListener(FIND_EVENT, open);
  }, []);

  const handleChange = useCallback(
    (markdown: string) => {
      if (!activeTab) return;
      dispatch({ type: "UPDATE_TAB_CONTENT", id: activeTab.id, content: markdown });
    },
    [activeTab, dispatch],
  );

  if (!activeTab) {
    return (
      <div className="flex h-full flex-1 flex-col items-center justify-center gap-3 text-sm text-muted">
        <Wordmark size={56} className="opacity-40" />
        Open a folder and select a markdown file to start editing.
      </div>
    );
  }

  return (
    <div className="relative flex h-full flex-1 min-w-0 flex-col">
      {search && (
        <Suspense fallback={null}>
          <SearchBar
            key={activeTab.id + activeTab.mode}
            mode={activeTab.mode}
            initialShowReplace={search.replace}
            containerRef={contentRef}
            onClose={() => setSearch(null)}
          />
        </Suspense>
      )}
      <div ref={contentRef} className="min-h-0 flex-1">
        {/* No spinner: the chunk is local and resolves in a frame or two, and a
            flash of anything here reads as the document itself flickering. */}
        <Suspense fallback={null}>
          {activeTab.mode === "rich" ? (
            <RichTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
          ) : (
            <PlainTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
          )}
        </Suspense>
      </div>
    </div>
  );
}
