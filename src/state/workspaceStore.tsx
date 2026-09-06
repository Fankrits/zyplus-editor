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
import { readProjectNode, writeTextFile } from "../lib/fs";
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
  /** Save dirty tabs automatically shortly after typing stops. */
  isAutosaveEnabled: boolean;
  setIsAutosaveEnabled: (enabled: boolean) => void;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(workspaceReducer, initialState);
  const [isHydrated, setIsHydrated] = useState(false);
  const [defaultFolder, setDefaultFolder] = useState<string | null>(null);
  const [isAutosaveEnabled, setIsAutosaveEnabled] = useState(false);

  useEffect(() => {
    let isMounted = true;
    async function hydrate() {
      const stored = loadSession();
      if (stored?.defaultFolder) setDefaultFolder(stored.defaultFolder);
      if (stored?.isAutosaveEnabled) setIsAutosaveEnabled(true);
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
      isAutosaveEnabled,
    });
  }, [isHydrated, state.roots, state.tabs, state.activeTabId, defaultFolder, isAutosaveEnabled]);

  useEffect(() => {
    if (!isAutosaveEnabled) return;
    const dirty = state.tabs.filter((t) => t.isDirty);
    if (dirty.length === 0) return;
    // Debounce restarts on every keystroke, so this fires once typing stops.
    const timer = setTimeout(() => {
      for (const tab of dirty) {
        writeTextFile(tab.filePath, tab.content)
          .then(() => dispatch({ type: "SAVE_TAB_SUCCESS", id: tab.id, content: tab.content }))
          .catch((err) => console.warn("Autosave failed for", tab.filePath, err));
      }
    }, 800);
    return () => clearTimeout(timer);
  }, [isAutosaveEnabled, state.tabs]);

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
    () => ({
      state,
      dispatch,
      activeTab,
      refreshTree,
      isHydrated,
      defaultFolder,
      setDefaultFolder,
      isAutosaveEnabled,
      setIsAutosaveEnabled,
    }),
    [state, activeTab, refreshTree, isHydrated, defaultFolder, isAutosaveEnabled],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used within a WorkspaceProvider");
  return ctx;
}
