import { readProjectNode, readTextFile } from "../lib/fs";
import type { PersistedWorkspaceSession } from "../lib/sessionStorage";

export type TabMode = "rich" | "plain";

export interface TreeNode {
  id: string;
  name: string;
  isFolder: boolean;
  children?: TreeNode[];
}

export interface TabState {
  id: string;
  filePath: string;
  title: string;
  content: string;
  isDirty: boolean;
  mode: TabMode;
}

export interface WorkspaceState {
  /** Open project folders, in sidebar order. Each is a top-level node in `tree`. */
  roots: string[];
  tree: TreeNode[];
  tabs: TabState[];
  activeTabId: string | null;
}

export type Action =
  | { type: "ADD_ROOT"; rootPath: string; node: TreeNode }
  | { type: "CLOSE_ROOT"; rootPath: string }
  | { type: "SET_TREE"; tree: TreeNode[] }
  | { type: "OPEN_TAB"; tab: TabState }
  | { type: "FOCUS_TAB"; id: string }
  | { type: "CLOSE_TAB"; id: string }
  | { type: "REORDER_TAB"; id: string; targetId: string | null }
  | { type: "UPDATE_TAB_CONTENT"; id: string; content: string }
  | { type: "SET_TAB_MODE"; id: string; mode: TabMode }
  | { type: "SAVE_TAB_SUCCESS"; id: string; content?: string }
  | { type: "REMAP_TAB_PATHS"; oldPrefix: string; newPrefix: string }
  | { type: "CLOSE_TABS_UNDER"; prefix: string }
  | {
      type: "RESTORE_WORKSPACE";
      roots: string[];
      tree: TreeNode[];
      tabs: TabState[];
      activeTabId: string | null;
    };

export const initialState: WorkspaceState = {
  roots: [],
  tree: [],
  tabs: [],
  activeTabId: null,
};

export function basenameOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function isUnder(path: string, prefix: string): boolean {
  return path === prefix || path.startsWith(prefix + "/") || path.startsWith(prefix + "\\");
}

export function nextActiveId(tabs: TabState[], closedId: string, prevActiveId: string | null) {
  if (prevActiveId !== closedId) return prevActiveId;
  const remaining = tabs.filter((t) => t.id !== closedId);
  return remaining.length > 0 ? remaining[remaining.length - 1].id : null;
}

export function workspaceReducer(state: WorkspaceState, action: Action): WorkspaceState {
  switch (action.type) {
    case "RESTORE_WORKSPACE":
      return {
        roots: action.roots,
        tree: action.tree,
        tabs: action.tabs,
        activeTabId: action.activeTabId,
      };
    case "ADD_ROOT": {
      if (state.roots.includes(action.rootPath)) return state;
      return {
        ...state,
        roots: [...state.roots, action.rootPath],
        tree: [...state.tree, action.node],
      };
    }
    case "CLOSE_ROOT": {
      const { rootPath } = action;
      const keep = state.tabs.filter((t) => !isUnder(t.filePath, rootPath));
      return {
        roots: state.roots.filter((r) => r !== rootPath),
        tree: state.tree.filter((n) => n.id !== rootPath),
        tabs: keep,
        activeTabId: keep.some((t) => t.id === state.activeTabId)
          ? state.activeTabId
          : (keep[keep.length - 1]?.id ?? null),
      };
    }
    case "SET_TREE":
      return { ...state, tree: action.tree };
    case "OPEN_TAB":
      return {
        ...state,
        tabs: [...state.tabs, action.tab],
        activeTabId: action.tab.id,
      };
    case "FOCUS_TAB":
      return { ...state, activeTabId: action.id };
    case "REORDER_TAB": {
      const { id, targetId } = action;
      if (id === targetId) return state;
      const tabs = [...state.tabs];
      const fromIndex = tabs.findIndex((t) => t.id === id);
      if (fromIndex === -1) return state;
      const [moved] = tabs.splice(fromIndex, 1);
      const toIndex = targetId ? tabs.findIndex((t) => t.id === targetId) : -1;
      if (toIndex === -1) {
        tabs.push(moved);
      } else {
        tabs.splice(toIndex, 0, moved);
      }
      return { ...state, tabs };
    }
    case "CLOSE_TAB":
      return {
        ...state,
        tabs: state.tabs.filter((t) => t.id !== action.id),
        activeTabId: nextActiveId(state.tabs, action.id, state.activeTabId),
      };
    case "UPDATE_TAB_CONTENT":
      return {
        ...state,
        tabs: state.tabs.map((t) =>
          t.id === action.id ? { ...t, content: action.content, isDirty: true } : t,
        ),
      };
    case "SET_TAB_MODE":
      return {
        ...state,
        tabs: state.tabs.map((t) => (t.id === action.id ? { ...t, mode: action.mode } : t)),
      };
    case "SAVE_TAB_SUCCESS":
      return {
        ...state,
        // `content` (autosave) keeps edits made while the write was in flight dirty.
        tabs: state.tabs.map((t) =>
          t.id === action.id && (action.content === undefined || action.content === t.content)
            ? { ...t, isDirty: false }
            : t,
        ),
      };
    case "REMAP_TAB_PATHS": {
      const { oldPrefix, newPrefix } = action;
      const tabs = state.tabs.map((t) => {
        if (t.filePath === oldPrefix) {
          return { ...t, id: newPrefix, filePath: newPrefix, title: basenameOf(newPrefix) };
        }
        if (t.filePath.startsWith(oldPrefix + "/") || t.filePath.startsWith(oldPrefix + "\\")) {
          const newFilePath = newPrefix + t.filePath.slice(oldPrefix.length);
          return { ...t, id: newFilePath, filePath: newFilePath };
        }
        return t;
      });
      const oldActiveIndex = state.tabs.findIndex((t) => t.id === state.activeTabId);
      const activeTabId = oldActiveIndex === -1 ? state.activeTabId : tabs[oldActiveIndex].id;
      return { ...state, tabs, activeTabId };
    }
    case "CLOSE_TABS_UNDER": {
      const { prefix } = action;
      const keep = state.tabs.filter((t) => !isUnder(t.filePath, prefix));
      const removedActive = !keep.some((t) => t.id === state.activeTabId);
      return {
        ...state,
        tabs: keep,
        activeTabId: removedActive ? (keep.length > 0 ? keep[keep.length - 1].id : null) : state.activeTabId,
      };
    }
    default:
      return state;
  }
}

export async function restoreWorkspaceFromSession(
  stored: PersistedWorkspaceSession | null,
  fsApi: {
    readProjectNode: (dir: string) => Promise<TreeNode>;
    readTextFile: (path: string) => Promise<string>;
  } = { readProjectNode, readTextFile },
): Promise<WorkspaceState | null> {
  if (!stored || stored.roots.length === 0) return null;

  const roots: string[] = [];
  const tree: TreeNode[] = [];
  for (const root of stored.roots) {
    try {
      tree.push(await fsApi.readProjectNode(root));
      roots.push(root);
    } catch {
      // Project folder is gone or unreadable — drop it silently.
    }
  }

  const tabs: TabState[] = [];
  for (const tab of stored.tabs) {
    try {
      const content = await fsApi.readTextFile(tab.filePath);
      tabs.push({
        id: tab.filePath,
        filePath: tab.filePath,
        title: basenameOf(tab.filePath),
        content,
        isDirty: false,
        mode: tab.mode,
      });
    } catch {
      // Gracefully skip tab if reading failed (e.g. file deleted/moved)
    }
  }

  let activeTabId: string | null = null;
  if (stored.activeFilePath && tabs.some((t) => t.id === stored.activeFilePath)) {
    activeTabId = stored.activeFilePath;
  } else if (tabs.length > 0) {
    activeTabId = tabs[tabs.length - 1].id;
  }

  return { roots, tree, tabs, activeTabId };
}
