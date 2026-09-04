import { memo, useCallback, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Button, Modal } from "@heroui/react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useWorkspace, type TabState } from "../../state/workspaceStore";
import { writeTextFile } from "../../lib/fs";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "../ContextMenu";

interface TabItemProps {
  tab: Pick<TabState, "id" | "title" | "isDirty">;
  isActive: boolean;
  onSelect: (id: string) => void;
  onRequestClose: (id: string) => void;
  onContextMenu: (e: ReactMouseEvent, id: string) => void;
}

const TabItem = memo(function TabItem({ tab, isActive, onSelect, onRequestClose, onContextMenu }: TabItemProps) {
  return (
    <div
      role="tab"
      aria-selected={isActive}
      onClick={() => onSelect(tab.id)}
      onContextMenu={(e) => onContextMenu(e, tab.id)}
      className={`group flex shrink-0 cursor-default items-center gap-2 border-r border-black/10 px-3 py-1.5 text-sm dark:border-white/10 ${
        isActive ? "bg-white dark:bg-neutral-800" : "text-neutral-500 hover:bg-black/5 dark:hover:bg-white/5"
      }`}
    >
      {tab.isDirty && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />}
      <span className="max-w-[14rem] truncate">{tab.title}</span>
      <button
        type="button"
        aria-label={`Close ${tab.title}`}
        onClick={(e) => {
          e.stopPropagation();
          onRequestClose(tab.id);
        }}
        className="rounded px-1 text-xs opacity-0 group-hover:opacity-100 hover:bg-black/10 dark:hover:bg-white/10"
      >
        ✕
      </button>
    </div>
  );
});

export function TabBar() {
  const { state, dispatch } = useWorkspace();
  const [pendingCloseId, setPendingCloseId] = useState<string | null>(null);
  const contextMenu = useContextMenu();

  const handleSelect = useCallback(
    (id: string) => dispatch({ type: "FOCUS_TAB", id }),
    [dispatch],
  );

  const handleRequestClose = useCallback(
    (id: string) => {
      const tab = state.tabs.find((t) => t.id === id);
      if (!tab) return;
      if (!tab.isDirty) {
        dispatch({ type: "CLOSE_TAB", id });
        return;
      }
      setPendingCloseId(id);
    },
    [state.tabs, dispatch],
  );

  const handleSave = useCallback(
    async (id: string) => {
      const tab = state.tabs.find((t) => t.id === id);
      if (!tab) return;
      await writeTextFile(tab.filePath, tab.content);
      dispatch({ type: "SAVE_TAB_SUCCESS", id });
    },
    [state.tabs, dispatch],
  );

  // Bulk close only ever affects clean tabs — dirty tabs are left open rather than
  // risking silently discarded work; use the single "Close" action to be prompted per-tab.
  const handleCloseOthers = useCallback(
    (keepId: string) => {
      state.tabs.filter((t) => t.id !== keepId && !t.isDirty).forEach((t) => dispatch({ type: "CLOSE_TAB", id: t.id }));
    },
    [state.tabs, dispatch],
  );

  const handleCloseAll = useCallback(() => {
    state.tabs.filter((t) => !t.isDirty).forEach((t) => dispatch({ type: "CLOSE_TAB", id: t.id }));
  }, [state.tabs, dispatch]);

  const handleTabContextMenu = useCallback(
    (e: ReactMouseEvent, id: string) => {
      const tab = state.tabs.find((t) => t.id === id);
      if (!tab) return;
      const items: ContextMenuItem[] = [
        { key: "save", label: "Save", disabled: !tab.isDirty, onSelect: () => handleSave(id) },
        { key: "close", label: "Close", onSelect: () => handleRequestClose(id) },
        { key: "close-others", label: "Close Others", onSelect: () => handleCloseOthers(id) },
        { key: "close-all", label: "Close All", onSelect: () => handleCloseAll() },
        { key: "reveal", label: "Reveal in Finder", onSelect: () => revealItemInDir(tab.filePath) },
        { key: "copy-path", label: "Copy Path", onSelect: () => navigator.clipboard.writeText(tab.filePath) },
      ];
      contextMenu.open(e, items);
    },
    [state.tabs, handleSave, handleRequestClose, handleCloseOthers, handleCloseAll, contextMenu],
  );

  const pendingTab = state.tabs.find((t) => t.id === pendingCloseId) ?? null;

  const handleDiscard = () => {
    if (pendingTab) dispatch({ type: "CLOSE_TAB", id: pendingTab.id });
    setPendingCloseId(null);
  };

  const handleSaveAndClose = async () => {
    if (pendingTab) {
      await writeTextFile(pendingTab.filePath, pendingTab.content);
      dispatch({ type: "SAVE_TAB_SUCCESS", id: pendingTab.id });
      dispatch({ type: "CLOSE_TAB", id: pendingTab.id });
    }
    setPendingCloseId(null);
  };

  if (state.tabs.length === 0) {
    return <div className="h-9 shrink-0 border-b border-black/10 dark:border-white/10" />;
  }

  return (
    <>
      <div
        role="tablist"
        className="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-black/10 dark:border-white/10"
      >
        {state.tabs.map((tab) => (
          <TabItem
            key={tab.id}
            tab={tab}
            isActive={tab.id === state.activeTabId}
            onSelect={handleSelect}
            onRequestClose={handleRequestClose}
            onContextMenu={handleTabContextMenu}
          />
        ))}
      </div>

      <ContextMenu state={contextMenu.state} onClose={contextMenu.close} />

      <Modal>
        <Modal.Backdrop isOpen={pendingTab !== null} onOpenChange={(open) => !open && setPendingCloseId(null)}>
          <Modal.Container>
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
                <Modal.Heading>Unsaved changes</Modal.Heading>
              </Modal.Header>
              <Modal.Body>
                <p>
                  "{pendingTab?.title}" has unsaved changes. Save before closing, or discard them?
                </p>
              </Modal.Body>
              <Modal.Footer>
                <Button variant="ghost" onPress={() => setPendingCloseId(null)}>
                  Cancel
                </Button>
                <Button variant="danger" onPress={handleDiscard}>
                  Discard
                </Button>
                <Button variant="primary" onPress={handleSaveAndClose}>
                  Save
                </Button>
              </Modal.Footer>
            </Modal.Dialog>
          </Modal.Container>
        </Modal.Backdrop>
      </Modal>
    </>
  );
}
