import { memo, useCallback, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Button, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  Cancel01Icon,
  FloppyDiskIcon,
  MultiplicationSignIcon,
  FolderOpenIcon,
  ClipboardIcon,
  Alert01Icon,
  PlusSignIcon,
  TextFontIcon,
  SourceCodeIcon,
  SidebarLeftIcon,
} from "@hugeicons/core-free-icons";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useWorkspace, type TabState } from "../../state/workspaceStore";
import { writeTextFile } from "../../lib/fs";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "../ContextMenu";

interface TabItemProps {
  tab: Pick<TabState, "id" | "title" | "isDirty">;
  isActive: boolean;
  isDragging: boolean;
  isDragOver: boolean;
  onSelect: (id: string) => void;
  onRequestClose: (id: string) => void;
  onContextMenu: (e: ReactMouseEvent, id: string) => void;
  onDragStartTab: (id: string) => void;
  onDragOverTab: (id: string) => void;
  onDropTab: (id: string) => void;
  onDragEndTab: () => void;
}

const TabItem = memo(function TabItem({
  tab,
  isActive,
  isDragging,
  isDragOver,
  onSelect,
  onRequestClose,
  onContextMenu,
  onDragStartTab,
  onDragOverTab,
  onDropTab,
  onDragEndTab,
}: TabItemProps) {
  return (
    <div
      role="tab"
      tabIndex={0}
      aria-selected={isActive}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", tab.id);
        onDragStartTab(tab.id);
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        onDragOverTab(tab.id);
      }}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onDropTab(tab.id);
      }}
      onDragEnd={onDragEndTab}
      onClick={() => onSelect(tab.id)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(tab.id);
        }
      }}
      onContextMenu={(e) => onContextMenu(e, tab.id)}
      className={`group flex h-8 shrink-0 cursor-default items-center gap-2 rounded-3xl px-3 text-sm font-medium no-highlight outline-none ${
        isActive ? "bg-accent-soft text-accent-soft-foreground" : "text-muted hover:opacity-70"
      } ${isDragging ? "opacity-40" : ""} ${isDragOver ? "status-focused" : ""} focus-visible:status-focused`}
    >
      {tab.isDirty && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-current" />}
      <span className="max-w-[8rem] sm:max-w-[14rem] truncate">{tab.title}</span>
      <button
        type="button"
        aria-label={`Close ${tab.title}`}
        onClick={(e) => {
          e.stopPropagation();
          onRequestClose(tab.id);
        }}
        className="rounded-full p-0.5 opacity-0 group-hover:opacity-100 hover:bg-foreground/10"
      >
        <HugeiconsIcon icon={Cancel01Icon} size={13} strokeWidth={2} />
      </button>
    </div>
  );
});

interface TabBarProps {
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  isSidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  isDesktop?: boolean;
  isMobileDrawerOpen?: boolean;
}

export function TabBar({
  onRequestCreate,
  isSidebarCollapsed,
  onToggleSidebar,
  isDesktop = true,
  isMobileDrawerOpen = false,
}: TabBarProps) {
  const { state, activeTab, dispatch, defaultFolder } = useWorkspace();
  const newFileDir =
    defaultFolder && state.roots.includes(defaultFolder) ? defaultFolder : state.roots[0];
  const [pendingCloseId, setPendingCloseId] = useState<string | null>(null);
  const [draggedId, setDraggedId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const contextMenu = useContextMenu();

  const handleDragOverTab = useCallback((id: string) => {
    setDragOverId((prev) => (prev === id ? prev : id));
  }, []);

  const handleDropOnTab = useCallback(
    (id: string) => {
      if (draggedId && draggedId !== id) {
        dispatch({ type: "REORDER_TAB", id: draggedId, targetId: id });
      }
      setDraggedId(null);
      setDragOverId(null);
    },
    [draggedId, dispatch],
  );

  const handleDragEnd = useCallback(() => {
    setDraggedId(null);
    setDragOverId(null);
  }, []);

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
        { key: "save", label: "Save", icon: FloppyDiskIcon, disabled: !tab.isDirty, onSelect: () => handleSave(id) },
        { key: "close", label: "Close", icon: Cancel01Icon, onSelect: () => handleRequestClose(id) },
        {
          key: "close-others",
          label: "Close Others",
          icon: MultiplicationSignIcon,
          onSelect: () => handleCloseOthers(id),
        },
        { key: "close-all", label: "Close All", icon: MultiplicationSignIcon, onSelect: () => handleCloseAll() },
        { key: "reveal", label: "Reveal in Finder", icon: FolderOpenIcon, onSelect: () => revealItemInDir(tab.filePath) },
        {
          key: "copy-path",
          label: "Copy Path",
          icon: ClipboardIcon,
          onSelect: () => navigator.clipboard.writeText(tab.filePath),
        },
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

  return (
    <>
      <div
        className="flex h-12 shrink-0 items-center gap-1 overflow-x-auto border-b border-border bg-white px-1.5 py-2"
        onDragOver={(e) => {
          if (e.target !== e.currentTarget) return;
          e.preventDefault();
          setDragOverId(null);
        }}
        onDrop={(e) => {
          if (e.target !== e.currentTarget) return;
          e.preventDefault();
          if (draggedId) dispatch({ type: "REORDER_TAB", id: draggedId, targetId: null });
          setDraggedId(null);
          setDragOverId(null);
        }}
      >
        <button
          type="button"
          aria-label={
            isDesktop
              ? isSidebarCollapsed
                ? "Show sidebar"
                : "Hide sidebar"
              : isMobileDrawerOpen
                ? "Close file drawer"
                : "Open file drawer"
          }
          onClick={onToggleSidebar}
          className={`flex size-8 shrink-0 items-center justify-center rounded-3xl no-highlight outline-none hover:opacity-70 focus-visible:status-focused ${
            (isDesktop ? !isSidebarCollapsed : isMobileDrawerOpen) ? "text-accent" : "text-muted"
          }`}
        >
          <HugeiconsIcon icon={SidebarLeftIcon} size={16} strokeWidth={2} />
        </button>
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto [scrollbar-width:none] [-ms-overflow-style:none] [&::-webkit-scrollbar]:hidden">
          <div role="tablist" className="flex shrink-0 items-center gap-1">
            {state.tabs.map((tab) => (
              <TabItem
                key={tab.id}
                tab={tab}
                isActive={tab.id === state.activeTabId}
                isDragging={tab.id === draggedId}
                isDragOver={tab.id === dragOverId}
                onSelect={handleSelect}
                onRequestClose={handleRequestClose}
                onContextMenu={handleTabContextMenu}
                onDragStartTab={setDraggedId}
                onDragOverTab={handleDragOverTab}
                onDropTab={handleDropOnTab}
                onDragEndTab={handleDragEnd}
              />
            ))}
          </div>
          <button
            type="button"
            aria-label="New file"
            disabled={newFileDir === undefined}
            onClick={() => newFileDir && onRequestCreate("file", newFileDir)}
            className="flex size-8 shrink-0 items-center justify-center rounded-3xl text-muted no-highlight outline-none hover:opacity-70 focus-visible:status-focused disabled:status-disabled"
          >
            <HugeiconsIcon icon={PlusSignIcon} size={16} strokeWidth={2} />
          </button>
        </div>
        {activeTab && (
          <div className="ml-auto flex shrink-0 items-center gap-1 pl-1">
            <Button
              size="sm"
              isIconOnly={!isDesktop}
              aria-label="Rich text editor"
              variant={activeTab.mode === "rich" ? "secondary" : "ghost"}
              onPress={() => dispatch({ type: "SET_TAB_MODE", id: activeTab.id, mode: "rich" })}
            >
              <HugeiconsIcon icon={TextFontIcon} size={15} />
              {isDesktop && "Rich"}
            </Button>
            <Button
              size="sm"
              isIconOnly={!isDesktop}
              aria-label="Plain text editor"
              variant={activeTab.mode === "plain" ? "secondary" : "ghost"}
              onPress={() => dispatch({ type: "SET_TAB_MODE", id: activeTab.id, mode: "plain" })}
            >
              <HugeiconsIcon icon={SourceCodeIcon} size={15} />
              {isDesktop && "Plain"}
            </Button>
          </div>
        )}
      </div>

      <ContextMenu state={contextMenu.state} onClose={contextMenu.close} />

      <Modal>
        <Modal.Backdrop isOpen={pendingTab !== null} onOpenChange={(open) => !open && setPendingCloseId(null)}>
          <Modal.Container>
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
                <Modal.Icon className="bg-warning-soft text-warning">
                  <HugeiconsIcon icon={Alert01Icon} size={20} />
                </Modal.Icon>
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
