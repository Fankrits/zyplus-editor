import { useCallback, useEffect, useRef, useState } from "react";
import { useWorkspace } from "../../state/workspaceStore";
import { RichTextEditor } from "./RichTextEditor";
import { PlainTextEditor } from "./PlainTextEditor";
import { SearchBar } from "./SearchBar";
import { Wordmark } from "../Logo";
import { FIND_EVENT } from "../../lib/commands";

export function EditorPane() {
  const { activeTab, dispatch } = useWorkspace();
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
        <SearchBar
          key={activeTab.id + activeTab.mode}
          mode={activeTab.mode}
          initialShowReplace={search.replace}
          containerRef={contentRef}
          onClose={() => setSearch(null)}
        />
      )}
      <div ref={contentRef} className="min-h-0 flex-1">
        {activeTab.mode === "rich" ? (
          <RichTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        ) : (
          <PlainTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        )}
      </div>
    </div>
  );
}
