import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Tree, type NodeApi, type NodeRendererProps } from "react-arborist";
import { dirname, join } from "@tauri-apps/api/path";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Button, Input, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  ArrowDown01Icon,
  ArrowRight01Icon,
  Folder01Icon,
  FolderOpenIcon,
  FolderAddIcon,
  File01Icon,
  FileTextIcon,
  FileAddIcon,
  PencilEdit01Icon,
  Copy01Icon,
  ExternalLinkIcon,
  ClipboardIcon,
  TrashIcon,
  Cancel01Icon,
  Alert01Icon,
} from "@hugeicons/core-free-icons";
import { useWorkspace, type TreeNode } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "../ContextMenu";

interface FileTreeProps {
  onOpenFile: (path: string, name: string) => void;
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  onOpenFolder?: () => void;
  onOpenFilePicker?: () => void;
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function FileTree({ onOpenFile, onRequestCreate, onOpenFolder, onOpenFilePicker }: FileTreeProps) {
  const { state, dispatch, refreshTree } = useWorkspace();
  const isRoot = useCallback((id: string) => state.roots.includes(id), [state.roots]);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [pendingDelete, setPendingDelete] = useState<NodeApi<TreeNode>[] | null>(null);
  const contextMenu = useContextMenu();

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      setSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const handleActivate = useCallback(
    (node: NodeApi<TreeNode>) => {
      if (node.data.isFolder) {
        node.toggle();
        return;
      }
      if (!fs.isMarkdownFile(node.data.name)) return;
      onOpenFile(node.data.id, node.data.name);
    },
    [onOpenFile],
  );

  const handleRename = useCallback(
    async ({ id, name }: { id: string; name: string }) => {
      const parent = await dirname(id);
      const newPath = await join(parent, name);
      if (newPath === id) return;
      await fs.renamePath(id, newPath);
      dispatch({ type: "REMAP_TAB_PATHS", oldPrefix: id, newPrefix: newPath });
      await refreshTree();
    },
    [dispatch, refreshTree],
  );

  const handleMove = useCallback(
    async ({ dragIds, parentId }: { dragIds: string[]; parentId: string | null }) => {
      // parentId === null means the workspace level, which holds projects, not files.
      if (!parentId) return;
      const destDir = parentId;
      for (const id of dragIds) {
        const newPath = await join(destDir, basename(id));
        if (newPath === id) continue;
        await fs.renamePath(id, newPath);
        dispatch({ type: "REMAP_TAB_PATHS", oldPrefix: id, newPrefix: newPath });
      }
      await refreshTree();
    },
    [dispatch, refreshTree],
  );

  // Keyboard-triggered delete (react-arborist's own Delete/Backspace handling) and the
  // context menu's "Delete" both just queue a confirmation instead of deleting outright.
  const handleDeleteRequest = useCallback(({ nodes }: { nodes: NodeApi<TreeNode>[] }) => {
    setPendingDelete(nodes);
  }, []);

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return;
    for (const node of pendingDelete) {
      await fs.deletePath(node.data.id, node.data.isFolder);
      dispatch({ type: "CLOSE_TABS_UNDER", prefix: node.data.id });
    }
    setPendingDelete(null);
    await refreshTree();
  }, [pendingDelete, dispatch, refreshTree]);

  const handleDuplicate = useCallback(
    async (node: NodeApi<TreeNode>) => {
      const newPath = await fs.duplicateFile(node.data.id);
      await refreshTree();
      onOpenFile(newPath, basename(newPath));
    },
    [refreshTree, onOpenFile],
  );

  const buildNodeMenuItems = useCallback(
    (node: NodeApi<TreeNode>): ContextMenuItem[] => {
      const isFile = !node.data.isFolder;
      const root = isRoot(node.data.id);
      const items: ContextMenuItem[] = [];

      if (isFile) {
        items.push({
          key: "open",
          label: "Open",
          icon: ExternalLinkIcon,
          onSelect: () => onOpenFile(node.data.id, node.data.name),
        });
      } else {
        items.push(
          { key: "new-file", label: "New File", icon: FileAddIcon, onSelect: () => onRequestCreate("file", node.data.id) },
          {
            key: "new-folder",
            label: "New Folder",
            icon: FolderAddIcon,
            onSelect: () => onRequestCreate("folder", node.data.id),
          },
        );
      }
      if (!root) {
        items.push({ key: "rename", label: "Rename", icon: PencilEdit01Icon, onSelect: () => node.edit() });
      }
      if (isFile) {
        items.push({ key: "duplicate", label: "Duplicate", icon: Copy01Icon, onSelect: () => handleDuplicate(node) });
      }
      items.push(
        { key: "reveal", label: "Reveal in Finder", icon: FolderOpenIcon, onSelect: () => revealItemInDir(node.data.id) },
        {
          key: "copy-path",
          label: "Copy Path",
          icon: ClipboardIcon,
          onSelect: () => navigator.clipboard.writeText(node.data.id),
        },
        root
          ? {
              key: "close-project",
              label: "Close Project",
              icon: Cancel01Icon,
              onSelect: () => dispatch({ type: "CLOSE_ROOT", rootPath: node.data.id }),
            }
          : { key: "delete", label: "Delete", icon: TrashIcon, danger: true, onSelect: () => setPendingDelete([node]) },
      );
      return items;
    },
    [onOpenFile, onRequestCreate, handleDuplicate, isRoot, dispatch],
  );

  const handleNodeContextMenu = useCallback(
    (e: ReactMouseEvent, node: NodeApi<TreeNode>) => {
      if (!node.isSelected) node.select();
      contextMenu.open(e, buildNodeMenuItems(node));
    },
    [contextMenu, buildNodeMenuItems],
  );

  // Right-click on empty space below the rows (not a row itself, since rows stop propagation).
  const handleTreeContextMenu = useCallback(
    (e: ReactMouseEvent) => {
      const primaryDir = state.roots[0];
      const items: ContextMenuItem[] = primaryDir
        ? [
            { key: "new-file", label: "New File", icon: FileAddIcon, onSelect: () => onRequestCreate("file", primaryDir) },
            {
              key: "new-folder",
              label: "New Folder",
              icon: FolderAddIcon,
              onSelect: () => onRequestCreate("folder", primaryDir),
            },
          ]
        : [];
      if (onOpenFilePicker) {
        items.push({
          key: "open-file",
          label: "Open File...",
          icon: File01Icon,
          onSelect: onOpenFilePicker,
        });
      }
      if (onOpenFolder) {
        items.push({
          key: "open-folder",
          label: "Add Project Folder...",
          icon: FolderOpenIcon,
          onSelect: onOpenFolder,
        });
      }
      contextMenu.open(e, items);
    },
    [state.roots, onRequestCreate, contextMenu, onOpenFilePicker, onOpenFolder],
  );

  const renderNode = useCallback(
    (props: NodeRendererProps<TreeNode>) => (
      <Node
        {...props}
        onNodeContextMenu={handleNodeContextMenu}
        onRequestCreate={onRequestCreate}
        isRoot={isRoot(props.node.data.id)}
      />
    ),
    [handleNodeContextMenu, onRequestCreate, isRoot],
  );

  return (
    <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden">
      <Tree<TreeNode>
        data={state.tree}
        width={size.width}
        height={size.height}
        rowHeight={40}
        indent={18}
        onActivate={handleActivate}
        onRename={handleRename}
        onMove={handleMove}
        onDelete={handleDeleteRequest}
        onContextMenu={handleTreeContextMenu}
        disableDrag={(data) => isRoot(data.id)}
      >
        {renderNode}
      </Tree>

      <ContextMenu state={contextMenu.state} onClose={contextMenu.close} />

      <Modal>
        <Modal.Backdrop isOpen={pendingDelete !== null} onOpenChange={(open) => !open && setPendingDelete(null)}>
          <Modal.Container>
            <Modal.Dialog>
              <Modal.CloseTrigger />
              <Modal.Header>
                <Modal.Icon className="bg-danger-soft text-danger">
                  <HugeiconsIcon icon={Alert01Icon} size={20} />
                </Modal.Icon>
                <Modal.Heading>
                  Delete {pendingDelete && pendingDelete.length > 1 ? `${pendingDelete.length} items` : pendingDelete?.[0]?.data.name}?
                </Modal.Heading>
              </Modal.Header>
              <Modal.Body>
                <p>This can&apos;t be undone.</p>
              </Modal.Body>
              <Modal.Footer>
                <Button variant="ghost" onPress={() => setPendingDelete(null)}>
                  Cancel
                </Button>
                <Button variant="danger" onPress={confirmDelete}>
                  Delete
                </Button>
              </Modal.Footer>
            </Modal.Dialog>
          </Modal.Container>
        </Modal.Backdrop>
      </Modal>
    </div>
  );
}

interface NodeProps extends NodeRendererProps<TreeNode> {
  onNodeContextMenu: (e: ReactMouseEvent, node: NodeApi<TreeNode>) => void;
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  isRoot: boolean;
}

function Node({ node, style, dragHandle, onNodeContextMenu, onRequestCreate, isRoot }: NodeProps) {
  // `style.paddingLeft` already equals node.level * the Tree's `indent` prop (react-arborist
  // computes this for us) — just add a small constant base inset on top of it.
  const rowStyle = { ...style, paddingLeft: (style.paddingLeft as number | undefined ?? 0) + 8 };

  if (node.isEditing) {
    const isFile = !node.data.isFolder;
    return (
      <div style={rowStyle} className="mx-1 my-0.5 flex h-[calc(100%-4px)] items-center gap-2 pr-2">
        <span className="w-3.5 shrink-0" />
        <HugeiconsIcon
          icon={
            node.data.isFolder
              ? node.isOpen
                ? FolderOpenIcon
                : Folder01Icon
              : isFile && fs.isMarkdownFile(node.data.name)
                ? FileTextIcon
                : File01Icon
          }
          size={16}
          strokeWidth={1.75}
          className="shrink-0 opacity-70"
        />
        <Input
          defaultValue={node.data.name}
          autoFocus
          fullWidth
          onFocus={(e) => {
            // Select just the filename stem (like Finder/VS Code) so retyping
            // doesn't clobber the extension by accident. Folders have no extension.
            const dotIndex = node.data.name.lastIndexOf(".");
            if (isFile && dotIndex > 0) {
              e.currentTarget.setSelectionRange(0, dotIndex);
            } else {
              e.currentTarget.select();
            }
          }}
          onBlur={() => node.reset()}
          onKeyDown={(e) => {
            if (e.key === "Escape") node.reset();
            if (e.key === "Enter") node.submit(e.currentTarget.value);
          }}
          className="h-7 px-2 py-0 text-sm"
        />
      </div>
    );
  }

  return (
    <div
      style={rowStyle}
      ref={dragHandle}
      onDoubleClick={() => (isRoot ? node.toggle() : node.edit())}
      onContextMenu={(e) => onNodeContextMenu(e, node)}
      className={`group mx-1 my-0.5 flex h-[calc(100%-4px)] cursor-default items-center gap-2 rounded-2xl pr-1 text-sm select-none ${
        node.isSelected ? "bg-accent-soft text-accent-soft-foreground" : "hover:bg-default"
      }`}
    >
      <span className="flex w-3.5 shrink-0 items-center justify-center opacity-60">
        {node.data.isFolder && (
          <HugeiconsIcon icon={node.isOpen ? ArrowDown01Icon : ArrowRight01Icon} size={13} strokeWidth={2} />
        )}
      </span>
      <HugeiconsIcon
        icon={
          node.data.isFolder
            ? node.isOpen
              ? FolderOpenIcon
              : Folder01Icon
            : fs.isMarkdownFile(node.data.name)
              ? FileTextIcon
              : File01Icon
        }
        size={16}
        strokeWidth={1.75}
        className="shrink-0 opacity-70"
      />
      <span className={`min-w-0 flex-1 truncate ${isRoot ? "font-semibold" : ""}`}>{node.data.name}</span>
      {node.data.isFolder && (
        <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <RowAction
            label="New file here"
            icon={FileAddIcon}
            onClick={() => {
              if (!node.isOpen) node.open();
              onRequestCreate("file", node.data.id);
            }}
          />
          <RowAction
            label="New folder here"
            icon={FolderAddIcon}
            onClick={() => {
              if (!node.isOpen) node.open();
              onRequestCreate("folder", node.data.id);
            }}
          />
        </span>
      )}
    </div>
  );
}

function RowAction({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon: typeof FileAddIcon;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      // Rows react to click/double-click, so keep row selection out of it.
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className="flex size-6 items-center justify-center rounded-lg opacity-70 hover:bg-surface-hover hover:opacity-100"
    >
      <HugeiconsIcon icon={icon} size={14} strokeWidth={1.75} />
    </button>
  );
}
