import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useState,
  type ReactNode,
} from "react";
import { readProjectNode } from "../lib/fs";
import { loadSession, saveSession } from "../lib/sessionStorage";
import {
  workspaceReducer,
  restoreWorkspaceFromSession,
  initialState,
  type Action,
  type TabMode,
  type TabState,
  type TreeNode,
  type WorkspaceState,
} from "./workspaceReducer";

export type { Action, TabMode, TabState, TreeNode, WorkspaceState };
export { basenameOf, workspaceReducer, restoreWorkspaceFromSession } from "./workspaceReducer";

interface WorkspaceContextValue {
  state: WorkspaceState;
  dispatch: React.Dispatch<Action>;
  activeTab: TabState | null;
  refreshTree: () => Promise<void>;
  /** False until the stored session has been read back. */
  isHydrated: boolean;
  /** Folder created on first run; where the header's "new file" lands. */
  defaultFolder: string | null;
  setDefaultFolder: (path: string) => void;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(workspaceReducer, initialState);
  const [isHydrated, setIsHydrated] = useState(false);
  const [defaultFolder, setDefaultFolder] = useState<string | null>(null);

  useEffect(() => {
    let isMounted = true;
    async function hydrate() {
      const stored = loadSession();
      if (stored?.defaultFolder) setDefaultFolder(stored.defaultFolder);
      if (stored && stored.roots.length > 0) {
        try {
          const restored = await restoreWorkspaceFromSession(stored);
          if (!isMounted) return;
          if (restored) {
            dispatch({
              type: "RESTORE_WORKSPACE",
              roots: restored.roots,
              tree: restored.tree,
              tabs: restored.tabs,
              activeTabId: restored.activeTabId,
            });
          }
        } catch (err) {
          console.warn("Failed to restore workspace session:", err);
          if (!isMounted) return;
          dispatch({
            type: "RESTORE_WORKSPACE",
            roots: [],
            tree: [],
            tabs: [],
            activeTabId: null,
          });
        }
      }

      if (isMounted) {
        setIsHydrated(true);
      }
    }

    hydrate();

    return () => {
      isMounted = false;
    };
  }, []);

  useEffect(() => {
    if (!isHydrated) return;
    const current = loadSession();
    saveSession({
      version: 2,
      roots: state.roots,
      defaultFolder,
      tabs: state.tabs.map((t) => ({ filePath: t.filePath, mode: t.mode })),
      activeFilePath: state.activeTabId,
      isSidebarCollapsed: current?.isSidebarCollapsed ?? false,
    });
  }, [isHydrated, state.roots, state.tabs, state.activeTabId, defaultFolder]);

  useEffect(() => {
    if (import.meta.env.DEV && typeof window !== "undefined") {
      (window as unknown as { __workspaceDispatch?: typeof dispatch }).__workspaceDispatch = dispatch;
      (window as unknown as { __workspaceState?: typeof state }).__workspaceState = state;
    }
  }, [dispatch, state]);

  const refreshTree = useCallback(async () => {
    if (state.roots.length === 0) return;
    const tree = await Promise.all(state.roots.map(readProjectNode));
    dispatch({ type: "SET_TREE", tree });
  }, [state.roots]);

  const activeTab = useMemo(
    () => state.tabs.find((t) => t.id === state.activeTabId) ?? null,
    [state.tabs, state.activeTabId],
  );

  const value = useMemo(
    () => ({ state, dispatch, activeTab, refreshTree, isHydrated, defaultFolder, setDefaultFolder }),
    [state, activeTab, refreshTree, isHydrated, defaultFolder],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used within a WorkspaceProvider");
  return ctx;
}
