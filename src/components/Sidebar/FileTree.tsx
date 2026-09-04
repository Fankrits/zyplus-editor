import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Tree, type NodeApi, type NodeRendererProps } from "react-arborist";
import { dirname, join } from "@tauri-apps/api/path";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Button, Modal } from "@heroui/react";
import { useWorkspace, type TreeNode } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "../ContextMenu";

interface FileTreeProps {
  onOpenFile: (path: string, name: string) => void;
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function FileTree({ onOpenFile, onRequestCreate }: FileTreeProps) {
  const { state, dispatch, refreshTree } = useWorkspace();
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
      const destDir = parentId ?? state.rootPath;
      if (!destDir) return;
      for (const id of dragIds) {
        const newPath = await join(destDir, basename(id));
        if (newPath === id) continue;
        await fs.renamePath(id, newPath);
        dispatch({ type: "REMAP_TAB_PATHS", oldPrefix: id, newPrefix: newPath });
      }
      await refreshTree();
    },
    [dispatch, refreshTree, state.rootPath],
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
      const items: ContextMenuItem[] = [];

      if (isFile) {
        items.push({ key: "open", label: "Open", onSelect: () => onOpenFile(node.data.id, node.data.name) });
      } else {
        items.push(
          { key: "new-file", label: "New File", onSelect: () => onRequestCreate("file", node.data.id) },
          { key: "new-folder", label: "New Folder", onSelect: () => onRequestCreate("folder", node.data.id) },
        );
      }
      items.push({ key: "rename", label: "Rename", onSelect: () => node.edit() });
      if (isFile) {
        items.push({ key: "duplicate", label: "Duplicate", onSelect: () => handleDuplicate(node) });
      }
      items.push(
        { key: "reveal", label: "Reveal in Finder", onSelect: () => revealItemInDir(node.data.id) },
        { key: "copy-path", label: "Copy Path", onSelect: () => navigator.clipboard.writeText(node.data.id) },
        { key: "delete", label: "Delete", danger: true, onSelect: () => setPendingDelete([node]) },
      );
      return items;
    },
    [onOpenFile, onRequestCreate, handleDuplicate],
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
      if (!state.rootPath) return;
      contextMenu.open(e, [
        { key: "new-file", label: "New File", onSelect: () => onRequestCreate("file", state.rootPath!) },
        { key: "new-folder", label: "New Folder", onSelect: () => onRequestCreate("folder", state.rootPath!) },
      ]);
    },
    [state.rootPath, onRequestCreate, contextMenu],
  );

  const renderNode = useCallback(
    (props: NodeRendererProps<TreeNode>) => <Node {...props} onNodeContextMenu={handleNodeContextMenu} />,
    [handleNodeContextMenu],
  );

  return (
    <div ref={containerRef} className="min-h-0 flex-1 overflow-hidden">
      <Tree<TreeNode>
        data={state.tree}
        width={size.width}
        height={size.height}
        rowHeight={26}
        indent={14}
        onActivate={handleActivate}
        onRename={handleRename}
        onMove={handleMove}
        onDelete={handleDeleteRequest}
        onContextMenu={handleTreeContextMenu}
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
}

function Node({ node, style, dragHandle, onNodeContextMenu }: NodeProps) {
  const indent = { ...style, paddingLeft: (style.paddingLeft as number | undefined ?? 0) + node.level * 14 };

  if (node.isEditing) {
    return (
      <div style={indent} className="flex items-center px-1">
        <input
          defaultValue={node.data.name}
          autoFocus
          onFocus={(e) => e.currentTarget.select()}
          onBlur={() => node.reset()}
          onKeyDown={(e) => {
            if (e.key === "Escape") node.reset();
            if (e.key === "Enter") node.submit(e.currentTarget.value);
          }}
          className="w-full rounded border border-neutral-400 bg-white px-1 text-sm text-black outline-none dark:bg-neutral-900 dark:text-white"
        />
      </div>
    );
  }

  return (
    <div
      style={indent}
      ref={dragHandle}
      onDoubleClick={() => node.edit()}
      onContextMenu={(e) => onNodeContextMenu(e, node)}
      className={`flex cursor-default items-center gap-1 truncate px-1 text-sm select-none ${
        node.isSelected ? "bg-blue-500/20" : "hover:bg-black/5 dark:hover:bg-white/10"
      }`}
    >
      <span className="w-3 shrink-0 text-center text-[10px] opacity-60">
        {node.data.isFolder ? (node.isOpen ? "▾" : "▸") : ""}
      </span>
      <span className="truncate">{node.data.name}</span>
    </div>
  );
}
