import { Component, lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useWorkspaceActions, useWorkspaceTabs } from "../../state/workspaceStore";
import { Wordmark } from "../Logo";
import { FIND_EVENT } from "../../lib/commands";
import { useFileEditorId } from "../../extensions/useExtensions";
import { ExtensionFileEditor } from "./ExtensionFileEditor";
import { readTextFile } from "../../lib/fs";

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

/**
 * A lazy editor chunk that fails to load — offline, or a deploy replaced the
 * old chunks — otherwise unmounts the whole app to a blank window. Keyed per
 * tab, so switching tabs tries again.
 */
class EditorErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(err: unknown) {
    console.error("The editor failed to load:", err);
  }
  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-muted">
        The editor could not be loaded.
        <button type="button" className="underline" onClick={() => window.location.reload()}>
          Reload
        </button>
      </div>
    );
  }
}

export function EditorPane() {
  const { activeTab } = useWorkspaceTabs();
  const { dispatch } = useWorkspaceActions();
  const contentRef = useRef<HTMLDivElement>(null);
  const [search, setSearch] = useState<{ replace: boolean } | null>(null);
  const fileEditorId = useFileEditorId(activeTab?.filePath);

  useEffect(() => {
    const open = (e: Event) => {
      setSearch({ replace: (e as CustomEvent<{ replace?: boolean }>).detail?.replace ?? false });
    };
    window.addEventListener(FIND_EVENT, open);
    return () => window.removeEventListener(FIND_EVENT, open);
  }, []);

  // A big tab that sat in the background let go of its text (see `evictBackgroundTabs`);
  // read it back now that it is showing. The reload also remounts the editor with it.
  const evictedId = activeTab?.evicted ? activeTab.id : null;
  const evictedPath = activeTab?.filePath;
  useEffect(() => {
    if (!evictedId || !evictedPath) return;
    let cancelled = false;
    readTextFile(evictedPath).then(
      (content) => {
        if (!cancelled) dispatch({ type: "RELOAD_TAB", id: evictedId, content });
      },
      (err) => {
        // Gone or unreadable since it was last shown: nothing to show, so close it.
        console.warn("Could not read the tab back:", evictedPath, err);
        if (!cancelled) dispatch({ type: "CLOSE_TAB", id: evictedId });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [evictedId, evictedPath, dispatch]);

  const handleChange = useCallback(
    (markdown: string) => {
      if (!activeTab) return;
      dispatch({ type: "UPDATE_TAB_CONTENT", id: activeTab.id, content: markdown });
    },
    [activeTab, dispatch],
  );

  const handleViewChange = useCallback(
    (view: string) => {
      if (!activeTab) return;
      dispatch({ type: "SET_TAB_VIEW", id: activeTab.id, view });
    },
    [activeTab, dispatch],
  );

  if (!activeTab) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 text-sm text-muted">
        <Wordmark size={56} className="opacity-40" />
        Open a folder and select a markdown file to start editing.
      </div>
    );
  }

  // Nothing to show until an evicted tab's text is back.
  if (activeTab.evicted) return <div className="min-h-0 flex-1" />;

  // The editors read their content once, at mount. A reload (another
  // tab) has to remount them, or the next keystroke writes the old text back.
  const editorKey = `${activeTab.id}#${activeTab.reloads ?? 0}`;

  return (
    // Not `h-full`: that is the whole column, so with the tab bar above it the pane overflowed the
    // shell by the bar's height. The shell clips that but can still be scrolled (a long paste's
    // scroll-into-view did), which pushed the tab bar off the top. `min-h-0` lets the pane fill the rest.
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
      {/* An extension's editor has its own search. */}
      {search && !fileEditorId && (
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
        <EditorErrorBoundary key={editorKey}>
          <Suspense fallback={null}>
            {fileEditorId ? (
              <ExtensionFileEditor
                key={editorKey}
                extensionId={fileEditorId}
                filePath={activeTab.filePath}
                initialValue={activeTab.content}
                onChange={handleChange}
                mode={activeTab.view}
                onModeChange={handleViewChange}
              />
            ) : activeTab.mode === "rich" ? (
              <RichTextEditor key={editorKey} initialValue={activeTab.content} onChange={handleChange} />
            ) : (
              <PlainTextEditor key={editorKey} initialValue={activeTab.content} onChange={handleChange} />
            )}
          </Suspense>
        </EditorErrorBoundary>
      </div>
    </div>
  );
}
