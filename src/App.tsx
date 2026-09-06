import { useCallback, useEffect, useState } from "react";
import { join } from "@tauri-apps/api/path";
import { Button, Drawer, Input, Label, Modal, TextField } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FileAddIcon, FolderAddIcon } from "@hugeicons/core-free-icons";
import "./App.css";
import { WorkspaceProvider, useWorkspace } from "./state/workspaceStore";
import * as fs from "./lib/fs";
import { loadSession, saveSession } from "./lib/sessionStorage";
import { Sidebar, SidebarContent } from "./components/Sidebar/Sidebar";
import { TabBar } from "./components/Tabs/TabBar";
import { EditorPane } from "./components/Editor/EditorPane";
import { Welcome } from "./components/Welcome";
import { SettingsModal, type SettingsSection } from "./components/SettingsModal";
import { useIsDesktop } from "./lib/useMediaQuery";
import { CLOSE_TAB_EVENT, FIND_EVENT, SETTINGS_EVENT, emit } from "./lib/commands";
import { isMac, matchShortcut, type CommandId } from "./lib/shortcuts";

type CreateKind = "file" | "folder" | null;

function AppShell() {
  const {
    state,
    activeTab,
    dispatch,
    refreshTree,
    isHydrated,
    defaultFolder,
    setDefaultFolder,
    openFile,
    addFolder,
    openFilePicker,
  } = useWorkspace();
  const isDesktop = useIsDesktop();
  const [isMobileDrawerOpen, setIsMobileDrawerOpen] = useState(false);
  const [createKind, setCreateKind] = useState<CreateKind>(null);
  const [createTargetDir, setCreateTargetDir] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [settingsSection, setSettingsSection] = useState<SettingsSection | null>(null);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(() => {
    return loadSession()?.isSidebarCollapsed ?? false;
  });

  useEffect(() => {
    if (isDesktop) {
      setIsMobileDrawerOpen(false);
    }
  }, [isDesktop]);

  useEffect(() => {
    const current = loadSession() ?? {
      version: 2 as const,
      roots: [],
      defaultFolder: null,
      tabs: [],
      activeFilePath: null,
      isSidebarCollapsed: false,
      isAutosaveEnabled: false,
    };
    saveSession({ ...current, isSidebarCollapsed });
  }, [isSidebarCollapsed]);

  const requestCreate = useCallback((kind: "file" | "folder", targetDir: string) => {
    setCreateTargetDir(targetDir);
    setCreateKind(kind);
  }, []);

  const stepTab = useCallback(
    (delta: number) => {
      const { tabs, activeTabId } = state;
      if (tabs.length < 2) return;
      const i = tabs.findIndex((t) => t.id === activeTabId);
      const next = tabs[(((i === -1 ? 0 : i) + delta) % tabs.length + tabs.length) % tabs.length];
      dispatch({ type: "FOCUS_TAB", id: next.id });
    },
    [state, dispatch],
  );

  const runCommand = useCallback(
    (id: CommandId) => {
      // Commands needing a document are no-ops without one.
      const tab = activeTab;
      switch (id) {
        case "save":
          if (!tab?.isDirty) return;
          fs.writeTextFile(tab.filePath, tab.content).then(() => {
            dispatch({ type: "SAVE_TAB_SUCCESS", id: tab.id });
          });
          return;
        case "export-md":
          if (tab) fs.saveFileAs(tab.title.replace(/\.[^.]+$/, "") + ".md", tab.content);
          return;
        case "export-pdf":
          window.print();
          return;
        case "new-file":
        case "new-folder": {
          const activeDir = tab?.filePath?.replace(/[\\/][^\\/]*$/, "");
          const targetDir = activeDir || defaultFolder || state.roots[0];
          if (targetDir) requestCreate(id === "new-file" ? "file" : "folder", targetDir);
          return;
        }
        case "open-folder":
          addFolder();
          return;
        case "open-file":
          openFilePicker();
          return;
        case "close-tab":
          emit(CLOSE_TAB_EVENT);
          return;
        case "next-tab":
          stepTab(1);
          return;
        case "prev-tab":
          stepTab(-1);
          return;
        case "toggle-sidebar":
          if (isDesktop) setIsSidebarCollapsed((v) => !v);
          else setIsMobileDrawerOpen((v) => !v);
          return;
        case "toggle-mode":
          if (tab) dispatch({ type: "SET_TAB_MODE", id: tab.id, mode: tab.mode === "rich" ? "plain" : "rich" });
          return;
        case "settings":
          setSettingsSection("general");
          return;
        case "shortcuts":
          setSettingsSection("shortcuts");
          return;
        case "find":
          if (tab) emit(FIND_EVENT, { replace: false });
          return;
        case "replace":
          if (tab) emit(FIND_EVENT, { replace: true });
          return;
        case "copy-markdown":
          if (tab) navigator.clipboard.writeText(tab.content);
          return;
        case "copy-path":
          if (tab) navigator.clipboard.writeText(tab.filePath);
          return;
      }
    },
    [activeTab, dispatch, defaultFolder, state.roots, requestCreate, addFolder, openFilePicker, stepTab, isDesktop],
  );

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // ⌘1–9 jumps to the nth tab, 9 being the last one.
      if ((isMac ? e.metaKey : e.ctrlKey) && !e.shiftKey && !e.altKey && /^Digit[1-9]$/.test(e.code)) {
        e.preventDefault();
        const n = Number(e.code.slice(5));
        const tab = n === 9 ? state.tabs[state.tabs.length - 1] : state.tabs[n - 1];
        if (tab) dispatch({ type: "FOCUS_TAB", id: tab.id });
        return;
      }
      const command = matchShortcut(e);
      if (!command) return;
      e.preventDefault();
      runCommand(command);
    };
    const openSettings = () => setSettingsSection("general");
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener(SETTINGS_EVENT, openSettings);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener(SETTINGS_EVENT, openSettings);
    };
  }, [runCommand, state.tabs, dispatch]);

  const handleOpenFileFromDrawer = useCallback(
    async (path: string, name: string) => {
      setIsMobileDrawerOpen(false);
      await openFile(path, name);
    },
    [openFile],
  );

  const closeCreateModal = () => {
    setCreateKind(null);
    setCreateTargetDir(null);
    setNewName("");
  };

  const handleFirstFolder = useCallback(
    async (folder: string) => {
      setDefaultFolder(folder);
      const node = await fs.readProjectNode(folder);
      dispatch({ type: "ADD_ROOT", rootPath: folder, node });
    },
    [dispatch, setDefaultFolder],
  );

  const handleCreate = async () => {
    const targetDir = createTargetDir ?? defaultFolder ?? state.roots[0];
    if (!targetDir || !newName.trim() || !createKind) return;
    const trimmed = newName.trim();
    const name = createKind === "file" && !/\.[^./\\]+$/.test(trimmed) ? `${trimmed}.md` : trimmed;
    const path = await join(targetDir, name);
    if (createKind === "file") {
      await fs.createFile(path);
      await refreshTree();
      dispatch({
        type: "OPEN_TAB",
        tab: { id: path, filePath: path, title: name, content: "", isDirty: false, mode: "rich" },
      });
    } else {
      await fs.createFolder(path);
      await refreshTree();
    }
    closeCreateModal();
  };

  if (!isHydrated) return <div className="h-screen w-screen bg-background" />;

  if (!defaultFolder && state.roots.length === 0) {
    return <Welcome onReady={handleFirstFolder} />;
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      {isDesktop && <Sidebar onRequestCreate={requestCreate} isCollapsed={isSidebarCollapsed} />}
      <div className="flex min-w-0 flex-1 flex-col">
        <TabBar
          onRequestCreate={requestCreate}
          isSidebarCollapsed={isSidebarCollapsed}
          isDesktop={isDesktop}
          isMobileDrawerOpen={isMobileDrawerOpen}
          onToggleSidebar={() => {
            if (isDesktop) {
              setIsSidebarCollapsed((v) => !v);
            } else {
              setIsMobileDrawerOpen((v) => !v);
            }
          }}
        />
        <EditorPane />
      </div>

      {!isDesktop && (
        <Drawer isOpen={isMobileDrawerOpen} onOpenChange={setIsMobileDrawerOpen}>
          <Drawer.Backdrop>
            <Drawer.Content placement="left" className="w-72 max-w-[85vw] h-full p-0">
              <Drawer.Dialog className="h-full flex flex-col bg-background text-foreground">
                <Drawer.Header className="flex items-center justify-between border-b border-border px-3 py-2">
                  <Drawer.Heading className="text-sm font-semibold truncate">Files</Drawer.Heading>
                  <Drawer.CloseTrigger />
                </Drawer.Header>
                <Drawer.Body className="flex-1 p-0 overflow-hidden">
                  <SidebarContent
                    onRequestCreate={(kind, targetDir) => {
                      setIsMobileDrawerOpen(false);
                      requestCreate(kind, targetDir);
                    }}
                    onOpenFile={handleOpenFileFromDrawer}
                    onOpenFolder={() => setIsMobileDrawerOpen(false)}
                  />
                </Drawer.Body>
              </Drawer.Dialog>
            </Drawer.Content>
          </Drawer.Backdrop>
        </Drawer>
      )}

      <SettingsModal
        isOpen={settingsSection !== null}
        section={settingsSection ?? "general"}
        onSectionChange={setSettingsSection}
        onClose={() => setSettingsSection(null)}
      />

      <Modal>
        <Modal.Backdrop isOpen={createKind !== null} onOpenChange={(open) => !open && closeCreateModal()}>
          <Modal.Container size="sm">
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
                <Modal.Icon className="size-8 bg-accent-soft text-accent">
                  <HugeiconsIcon icon={createKind === "folder" ? FolderAddIcon : FileAddIcon} size={16} />
                </Modal.Icon>
                <Modal.Heading>{createKind === "folder" ? "New Folder" : "New File"}</Modal.Heading>
              </Modal.Header>
              <Modal.Body>
                <TextField>
                  <Label className="mb-1.5 block">{createKind === "folder" ? "Folder name" : "File name"}</Label>
                  <Input
                    value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    placeholder={createKind === "folder" ? "folder-name" : "file-name.md"}
                    autoFocus
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleCreate();
                    }}
                  />
                </TextField>
              </Modal.Body>
              <Modal.Footer>
                <Button size="sm" variant="ghost" onPress={closeCreateModal}>
                  Cancel
                </Button>
                <Button size="sm" variant="primary" onPress={handleCreate}>
                  Create
                </Button>
              </Modal.Footer>
            </Modal.Dialog>
          </Modal.Container>
        </Modal.Backdrop>
      </Modal>
    </div>
  );
}

function App() {
  return (
    <WorkspaceProvider>
      <AppShell />
    </WorkspaceProvider>
  );
}

export default App;
