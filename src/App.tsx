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
import { useIsDesktop } from "./lib/useMediaQuery";

type CreateKind = "file" | "folder" | null;

function AppShell() {
  const { state, activeTab, dispatch, refreshTree, isHydrated, defaultFolder, setDefaultFolder } =
    useWorkspace();
  const isDesktop = useIsDesktop();
  const [isMobileDrawerOpen, setIsMobileDrawerOpen] = useState(false);
  const [createKind, setCreateKind] = useState<CreateKind>(null);
  const [createTargetDir, setCreateTargetDir] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
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

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!e.metaKey && !e.ctrlKey) return;
      if (e.key === "s") {
        e.preventDefault();
        if (!activeTab || !activeTab.isDirty) return;
        fs.writeTextFile(activeTab.filePath, activeTab.content).then(() => {
          dispatch({ type: "SAVE_TAB_SUCCESS", id: activeTab.id });
        });
      } else if (e.key === "n") {
        e.preventDefault();
        const activeDir = activeTab?.filePath?.replace(/[\\/][^\\/]*$/, "");
        const targetDir = activeDir || defaultFolder || state.roots[0];
        if (targetDir) requestCreate("file", targetDir);
      } else if (e.key === "b") {
        e.preventDefault();
        if (isDesktop) setIsSidebarCollapsed((v) => !v);
        else setIsMobileDrawerOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeTab, dispatch, defaultFolder, state.roots, isDesktop, requestCreate]);

  const handleOpenFileFromDrawer = useCallback(
    async (path: string, name: string) => {
      setIsMobileDrawerOpen(false);
      const existing = state.tabs.find((t) => t.filePath === path);
      if (existing) {
        dispatch({ type: "FOCUS_TAB", id: existing.id });
        return;
      }
      const content = await fs.readTextFile(path);
      dispatch({
        type: "OPEN_TAB",
        tab: { id: path, filePath: path, title: name, content, isDirty: false, mode: "rich" },
      });
    },
    [state.tabs, dispatch],
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
