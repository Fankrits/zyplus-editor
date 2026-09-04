import { useCallback, useEffect, useState } from "react";
import { join } from "@tauri-apps/api/path";
import { Button, Input, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FileAddIcon, FolderAddIcon } from "@hugeicons/core-free-icons";
import "./App.css";
import { WorkspaceProvider, useWorkspace } from "./state/workspaceStore";
import * as fs from "./lib/fs";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { TabBar } from "./components/Tabs/TabBar";
import { EditorPane } from "./components/Editor/EditorPane";

type CreateKind = "file" | "folder" | null;

function AppShell() {
  const { state, activeTab, dispatch, refreshTree } = useWorkspace();
  const [createKind, setCreateKind] = useState<CreateKind>(null);
  const [createTargetDir, setCreateTargetDir] = useState<string | null>(null);
  const [newName, setNewName] = useState("");

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        if (!activeTab || !activeTab.isDirty) return;
        fs.writeTextFile(activeTab.filePath, activeTab.content).then(() => {
          dispatch({ type: "SAVE_TAB_SUCCESS", id: activeTab.id });
        });
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeTab, dispatch]);

  const requestCreate = useCallback((kind: "file" | "folder", targetDir: string) => {
    setCreateTargetDir(targetDir);
    setCreateKind(kind);
  }, []);

  const closeCreateModal = () => {
    setCreateKind(null);
    setCreateTargetDir(null);
    setNewName("");
  };

  const handleCreate = async () => {
    const targetDir = createTargetDir ?? state.rootPath;
    if (!targetDir || !newName.trim() || !createKind) return;
    const path = await join(targetDir, newName.trim());
    if (createKind === "file") {
      await fs.createFile(path);
      await refreshTree();
      dispatch({
        type: "OPEN_TAB",
        tab: { id: path, filePath: path, title: newName.trim(), content: "", isDirty: false, mode: "rich" },
      });
    } else {
      await fs.createFolder(path);
      await refreshTree();
    }
    closeCreateModal();
  };

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-white text-black dark:bg-neutral-900 dark:text-white">
      <Sidebar onRequestCreate={requestCreate} />
      <div className="flex min-w-0 flex-1 flex-col">
        <TabBar onRequestCreate={requestCreate} />
        <EditorPane />
      </div>

      <Modal>
        <Modal.Backdrop isOpen={createKind !== null} onOpenChange={(open) => !open && closeCreateModal()}>
          <Modal.Container>
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
                <Modal.Icon className="bg-accent-soft text-accent">
                  <HugeiconsIcon icon={createKind === "folder" ? FolderAddIcon : FileAddIcon} size={20} />
                </Modal.Icon>
                <Modal.Heading>{createKind === "folder" ? "New Folder" : "New File"}</Modal.Heading>
              </Modal.Header>
              <Modal.Body>
                <Input
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder={createKind === "folder" ? "folder-name" : "file-name.md"}
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreate();
                  }}
                />
              </Modal.Body>
              <Modal.Footer>
                <Button variant="ghost" onPress={closeCreateModal}>
                  Cancel
                </Button>
                <Button variant="primary" onPress={handleCreate}>
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
