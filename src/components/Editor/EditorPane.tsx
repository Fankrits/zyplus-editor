import { useCallback } from "react";
import { Button } from "@heroui/react";
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
      <div className="flex h-full flex-1 items-center justify-center text-sm text-neutral-500">
        Open a folder and select a markdown file to start editing.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-1 min-w-0 flex-col">
      <div className="flex shrink-0 items-center justify-end gap-1 border-b border-black/10 px-3 py-1.5 dark:border-white/10">
        <Button
          size="sm"
          variant={activeTab.mode === "rich" ? "secondary" : "ghost"}
          onPress={() => dispatch({ type: "SET_TAB_MODE", id: activeTab.id, mode: "rich" })}
        >
          Rich
        </Button>
        <Button
          size="sm"
          variant={activeTab.mode === "plain" ? "secondary" : "ghost"}
          onPress={() => dispatch({ type: "SET_TAB_MODE", id: activeTab.id, mode: "plain" })}
        >
          Plain
        </Button>
      </div>
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
