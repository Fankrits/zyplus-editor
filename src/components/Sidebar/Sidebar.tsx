import { useCallback, useState } from "react";
import { Button } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  FolderOpenIcon,
  FileAddIcon,
  FolderAddIcon,
  File01Icon,
  Settings01Icon,
} from "@hugeicons/core-free-icons";
import { useWorkspace } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";
import { FileTree } from "./FileTree";
import { Logo, Wordmark } from "../Logo";
import { SettingsModal } from "../SettingsModal";

export interface SidebarContentProps {
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  onOpenFile?: (path: string, name: string) => void;
  onOpenFolder?: () => void;
}

export function SidebarContent({ onRequestCreate, onOpenFile, onOpenFolder }: SidebarContentProps) {
  const { state, dispatch, activeTab, defaultFolder } = useWorkspace();
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);

  const handleOpenFolder = useCallback(async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    const node = await fs.readProjectNode(picked);
    dispatch({ type: "ADD_ROOT", rootPath: picked, node });
    onOpenFolder?.();
  }, [dispatch, onOpenFolder]);

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

  const handleOpenFilePicker = useCallback(async () => {
    const picked = await fs.openFileDialog();
    if (!picked) return;
    const name = picked.split(/[\\/]/).pop() ?? picked;
    handleOpenFile(picked, name);
  }, [handleOpenFile]);

  // Header-level "new file/folder" land next to the file you're editing; falling back to
  // the default project folder, then the first open one.
  const activeDir = activeTab?.filePath?.replace(/[\\/][^\\/]*$/, "");
  const primaryDir =
    activeDir ||
    (defaultFolder && state.roots.includes(defaultFolder) ? defaultFolder : state.roots[0]);

  if (state.roots.length === 0) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-4">
        <Wordmark size={40} className="mb-2" />
        <Button variant="primary" onPress={handleOpenFolder}>
          <HugeiconsIcon icon={FolderOpenIcon} size={18} />
          Open Folder
        </Button>
        <Button variant="ghost" size="sm" onPress={handleOpenFilePicker}>
          <HugeiconsIcon icon={File01Icon} size={16} />
          Open File
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex shrink-0 items-center justify-between gap-1 px-2 py-2">
        <span className="flex min-w-0 items-center gap-2">
          <Logo size={16} className="shrink-0 text-foreground" />
          <span className="truncate text-xs font-medium uppercase tracking-wide text-muted">
            Projects
          </span>
        </span>
        <div className="flex shrink-0 gap-1">
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="Open file"
            onPress={handleOpenFilePicker}
          >
            <HugeiconsIcon icon={File01Icon} size={16} />
          </Button>
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="Add project folder"
            onPress={handleOpenFolder}
          >
            <HugeiconsIcon icon={FolderOpenIcon} size={16} />
          </Button>
        </div>
      </div>
      <div className="flex shrink-0 gap-2 px-2 pb-2">
        <Button
          size="sm"
          variant="primary"
          className="flex-1"
          onPress={() => onRequestCreate("file", primaryDir)}
        >
          <HugeiconsIcon icon={FileAddIcon} size={16} />
          New File
        </Button>
        <Button
          size="sm"
          variant="secondary"
          isIconOnly
          aria-label="New folder"
          onPress={() => onRequestCreate("folder", primaryDir)}
        >
          <HugeiconsIcon icon={FolderAddIcon} size={16} />
        </Button>
      </div>
      <FileTree
        onOpenFile={handleOpenFile}
        onRequestCreate={onRequestCreate}
        onOpenFolder={handleOpenFolder}
        onOpenFilePicker={handleOpenFilePicker}
      />
      <div className="mt-auto shrink-0 border-t border-border p-2">
        <Button
          size="sm"
          variant="ghost"
          className="w-full justify-start"
          onPress={() => setIsSettingsOpen(true)}
        >
          <HugeiconsIcon icon={Settings01Icon} size={16} />
          Settings
        </Button>
      </div>
      <SettingsModal isOpen={isSettingsOpen} onClose={() => setIsSettingsOpen(false)} />
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
    <aside className="hidden md:flex h-full w-64 shrink-0 flex-col border-r border-border">
      <SidebarContent onRequestCreate={onRequestCreate} />
    </aside>
  );
}
