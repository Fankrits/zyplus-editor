import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useReducer,
  type ReactNode,
} from "react";
import { readDirRecursive } from "../lib/fs";

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
  rootPath: string | null;
  tree: TreeNode[];
  tabs: TabState[];
  activeTabId: string | null;
}

type Action =
  | { type: "OPEN_ROOT"; rootPath: string; tree: TreeNode[] }
  | { type: "SET_TREE"; tree: TreeNode[] }
  | { type: "OPEN_TAB"; tab: TabState }
  | { type: "FOCUS_TAB"; id: string }
  | { type: "CLOSE_TAB"; id: string }
  | { type: "UPDATE_TAB_CONTENT"; id: string; content: string }
  | { type: "SET_TAB_MODE"; id: string; mode: TabMode }
  | { type: "SAVE_TAB_SUCCESS"; id: string }
  | { type: "REMAP_TAB_PATHS"; oldPrefix: string; newPrefix: string }
  | { type: "CLOSE_TABS_UNDER"; prefix: string };

const initialState: WorkspaceState = {
  rootPath: null,
  tree: [],
  tabs: [],
  activeTabId: null,
};

function basenameOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

function nextActiveId(tabs: TabState[], closedId: string, prevActiveId: string | null) {
  if (prevActiveId !== closedId) return prevActiveId;
  const remaining = tabs.filter((t) => t.id !== closedId);
  return remaining.length > 0 ? remaining[remaining.length - 1].id : null;
}

function reducer(state: WorkspaceState, action: Action): WorkspaceState {
  switch (action.type) {
    case "OPEN_ROOT":
      return {
        rootPath: action.rootPath,
        tree: action.tree,
        tabs: [],
        activeTabId: null,
      };
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
        tabs: state.tabs.map((t) => (t.id === action.id ? { ...t, isDirty: false } : t)),
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
      const keep = state.tabs.filter(
        (t) =>
          t.filePath !== prefix &&
          !t.filePath.startsWith(prefix + "/") &&
          !t.filePath.startsWith(prefix + "\\"),
      );
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

interface WorkspaceContextValue {
  state: WorkspaceState;
  dispatch: React.Dispatch<Action>;
  activeTab: TabState | null;
  refreshTree: () => Promise<void>;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);

  const refreshTree = useCallback(async () => {
    if (!state.rootPath) return;
    const tree = await readDirRecursive(state.rootPath);
    dispatch({ type: "SET_TREE", tree });
  }, [state.rootPath]);

  const activeTab = useMemo(
    () => state.tabs.find((t) => t.id === state.activeTabId) ?? null,
    [state.tabs, state.activeTabId],
  );

  const value = useMemo(
    () => ({ state, dispatch, activeTab, refreshTree }),
    [state, activeTab, refreshTree],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used within a WorkspaceProvider");
  return ctx;
}
