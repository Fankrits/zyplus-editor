import { useCallback } from "react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FileEmpty01Icon } from "@hugeicons/core-free-icons";
import { useWorkspace } from "../../state/workspaceStore";
import { RichTextEditor } from "./RichTextEditor";
import { PlainTextEditor } from "./PlainTextEditor";

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
        <HugeiconsIcon icon={FileEmpty01Icon} size={32} strokeWidth={1.5} className="opacity-40" />
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
