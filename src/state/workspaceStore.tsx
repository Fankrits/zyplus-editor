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
import { readDirRecursive } from "../lib/fs";
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
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(workspaceReducer, initialState);
  const [isHydrated, setIsHydrated] = useState(false);

  useEffect(() => {
    let isMounted = true;
    async function hydrate() {
      const stored = loadSession();
      if (stored?.rootPath) {
        try {
          const restored = await restoreWorkspaceFromSession(stored);
          if (!isMounted) return;
          if (restored) {
            dispatch({
              type: "RESTORE_WORKSPACE",
              rootPath: restored.rootPath,
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
            rootPath: null,
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
      version: 1,
      rootPath: state.rootPath,
      tabs: state.tabs.map((t) => ({ filePath: t.filePath, mode: t.mode })),
      activeFilePath: state.activeTabId,
      isSidebarCollapsed: current?.isSidebarCollapsed ?? false,
    });
  }, [isHydrated, state.rootPath, state.tabs, state.activeTabId]);

  useEffect(() => {
    if (import.meta.env.DEV && typeof window !== "undefined") {
      (window as unknown as { __workspaceDispatch?: typeof dispatch }).__workspaceDispatch = dispatch;
      (window as unknown as { __workspaceState?: typeof state }).__workspaceState = state;
    }
  }, [dispatch, state]);

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
