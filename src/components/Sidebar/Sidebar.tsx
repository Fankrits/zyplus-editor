import { useCallback } from "react";
import { Button } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FolderOpenIcon, FileAddIcon, FolderAddIcon } from "@hugeicons/core-free-icons";
import { useWorkspace } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";
import { FileTree } from "./FileTree";

export interface SidebarContentProps {
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  onOpenFile?: (path: string, name: string) => void;
}

export function SidebarContent({ onRequestCreate, onOpenFile }: SidebarContentProps) {
  const { state, dispatch } = useWorkspace();

  const handleOpenFolder = useCallback(async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    const tree = await fs.readDirRecursive(picked);
    dispatch({ type: "OPEN_ROOT", rootPath: picked, tree });
  }, [dispatch]);

  const defaultOpenFile = useCallback(
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

  const handleOpenFile = onOpenFile ?? defaultOpenFile;

  if (!state.rootPath) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-4">
        <Button variant="primary" onPress={handleOpenFolder}>
          <HugeiconsIcon icon={FolderOpenIcon} size={18} />
          Open Folder
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
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
            onPress={() => onRequestCreate("file", state.rootPath!)}
          >
            <HugeiconsIcon icon={FileAddIcon} size={16} />
          </Button>
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="New folder"
            onPress={() => onRequestCreate("folder", state.rootPath!)}
          >
            <HugeiconsIcon icon={FolderAddIcon} size={16} />
          </Button>
        </div>
      </div>
      <FileTree onOpenFile={handleOpenFile} onRequestCreate={onRequestCreate} />
    </div>
  );
}

export interface SidebarProps {
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  isCollapsed: boolean;
}

export function Sidebar({ onRequestCreate, isCollapsed }: SidebarProps) {
  if (isCollapsed) return null;
  return (
    <aside className="hidden md:flex h-full w-64 shrink-0 flex-col border-r border-black/10 dark:border-white/10">
      <SidebarContent onRequestCreate={onRequestCreate} />
    </aside>
  );
}
