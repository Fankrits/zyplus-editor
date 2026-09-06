import { useCallback, useEffect, useRef, useState } from "react";
import { useWorkspace } from "../../state/workspaceStore";
import { RichTextEditor } from "./RichTextEditor";
import { PlainTextEditor } from "./PlainTextEditor";
import { SearchBar } from "./SearchBar";
import { Wordmark } from "../Logo";

/** Anything that wants to open the find bar fires this on `window`. */
export const FIND_EVENT = "zyplus:find";

export function EditorPane() {
  const { activeTab, dispatch } = useWorkspace();
  const contentRef = useRef<HTMLDivElement>(null);
  const [isSearchOpen, setIsSearchOpen] = useState(false);

  useEffect(() => {
    const open = () => setIsSearchOpen(true);
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        open();
      }
    };
    window.addEventListener(FIND_EVENT, open);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener(FIND_EVENT, open);
      window.removeEventListener("keydown", onKeyDown);
    };
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
      {isSearchOpen && (
        <SearchBar
          key={activeTab.id + activeTab.mode}
          mode={activeTab.mode}
          containerRef={contentRef}
          onClose={() => setIsSearchOpen(false)}
        />
      )}
      <div ref={contentRef} className="print-target min-h-0 flex-1">
        {activeTab.mode === "rich" ? (
          <RichTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        ) : (
          <PlainTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        )}
      </div>
    </div>
  );
}
