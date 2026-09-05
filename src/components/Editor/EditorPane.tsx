import { useCallback } from "react";
import { useWorkspace } from "../../state/workspaceStore";
import { RichTextEditor } from "./RichTextEditor";
import { PlainTextEditor } from "./PlainTextEditor";
import { Logo } from "../Logo";

export function EditorPane() {
  const { activeTab, dispatch } = useWorkspace();

  const handleChange = useCallback(
    (markdown: string) => {
      if (!activeTab) return;
      dispatch({ type: "UPDATE_TAB_CONTENT", id: activeTab.id, content: markdown });
    },
    [activeTab, dispatch],
  );

  if (!activeTab) {
    return (
      <div className="flex h-full flex-1 flex-col items-center justify-center gap-3 text-sm text-neutral-500">
        <Logo size={56} className="text-black opacity-15 dark:text-white" />
        <span className="text-base font-semibold tracking-tight text-black dark:text-white">Zyplus</span>
        Open a folder and select a markdown file to start editing.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-1 min-w-0 flex-col">
      <div className="min-h-0 flex-1">
        {activeTab.mode === "rich" ? (
          <RichTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        ) : (
          <PlainTextEditor key={activeTab.id} initialValue={activeTab.content} onChange={handleChange} />
        )}
      </div>
    </div>
  );
}
