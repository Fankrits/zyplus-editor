import { useCallback, useState } from "react";
import { join } from "@tauri-apps/api/path";
import { Button, Input, Modal } from "@heroui/react";
import { useWorkspace } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";
import { FileTree } from "./FileTree";

type CreateKind = "file" | "folder" | null;

export function Sidebar() {
  const { state, dispatch, refreshTree } = useWorkspace();
  const [createKind, setCreateKind] = useState<CreateKind>(null);
  const [createTargetDir, setCreateTargetDir] = useState<string | null>(null);
  const [newName, setNewName] = useState("");

  const requestCreate = useCallback((kind: "file" | "folder", targetDir: string) => {
    setCreateTargetDir(targetDir);
    setCreateKind(kind);
  }, []);

  const handleOpenFolder = useCallback(async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    const tree = await fs.readDirRecursive(picked);
    dispatch({ type: "OPEN_ROOT", rootPath: picked, tree });
  }, [dispatch]);

  const handleOpenFile = useCallback(
    async (path: string, name: string) => {
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

  const handleCreate = async () => {
    const targetDir = createTargetDir ?? state.rootPath;
    if (!targetDir || !newName.trim() || !createKind) return;
    const path = await join(targetDir, newName.trim());
    if (createKind === "file") {
      await fs.createFile(path);
    } else {
      await fs.createFolder(path);
    }
    await refreshTree();
    closeCreateModal();
  };

  if (!state.rootPath) {
    return (
      <div className="flex h-full w-64 shrink-0 flex-col items-center justify-center gap-3 border-r border-black/10 p-4 dark:border-white/10">
        <Button variant="primary" onPress={handleOpenFolder}>
          Open Folder
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-full w-64 shrink-0 flex-col border-r border-black/10 dark:border-white/10">
      <div className="flex shrink-0 items-center justify-between gap-1 px-2 py-2">
        <span className="truncate text-xs font-medium uppercase tracking-wide text-neutral-500">
          {state.rootPath.split(/[\\/]/).pop()}
        </span>
        <div className="flex shrink-0 gap-1">
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="New file"
            onPress={() => requestCreate("file", state.rootPath!)}
          >
            +
          </Button>
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="New folder"
            onPress={() => requestCreate("folder", state.rootPath!)}
          >
            /
          </Button>
        </div>
      </div>
      <FileTree onOpenFile={handleOpenFile} onRequestCreate={requestCreate} />

      <Modal>
        <Modal.Backdrop isOpen={createKind !== null} onOpenChange={(open) => !open && closeCreateModal()}>
          <Modal.Container>
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
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
