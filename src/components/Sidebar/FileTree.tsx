import { useCallback, useEffect, useRef, useState } from "react";
import { Tree, type NodeApi, type NodeRendererProps } from "react-arborist";
import { dirname, join } from "@tauri-apps/api/path";
import { useWorkspace, type TreeNode } from "../../state/workspaceStore";
import * as fs from "../../lib/fs";

interface FileTreeProps {
  onOpenFile: (path: string, name: string) => void;
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function FileTree({ onOpenFile }: FileTreeProps) {
  const { state, dispatch, refreshTree } = useWorkspace();
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });

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

  const handleDelete = useCallback(
    async ({ nodes }: { nodes: NodeApi<TreeNode>[] }) => {
      for (const node of nodes) {
        await fs.deletePath(node.data.id, node.data.isFolder);
        dispatch({ type: "CLOSE_TABS_UNDER", prefix: node.data.id });
      }
      await refreshTree();
    },
    [dispatch, refreshTree],
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
        onDelete={handleDelete}
      >
        {Node}
      </Tree>
    </div>
  );
}

function Node({ node, style, dragHandle }: NodeRendererProps<TreeNode>) {
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
