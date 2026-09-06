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
import {
  openFileDialog,
  openFolderDialog,
  readProjectNode,
  readTextFile,
  writeTextFile,
} from "../lib/fs";
import { loadSession, saveSession } from "../lib/sessionStorage";
import {
  basenameOf,
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
  /** Opens a file in a tab, focusing it if already open. */
  openFile: (path: string, name?: string) => Promise<void>;
  /** Prompts for a folder and adds it as a project root. Returns the picked path. */
  addFolder: () => Promise<string | null>;
  /** Prompts for a file and opens it in a tab. Returns the picked path. */
  openFilePicker: () => Promise<string | null>;
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

  const openFile = useCallback(
    async (path: string, name?: string) => {
      const existing = state.tabs.find((t) => t.filePath === path);
      if (existing) {
        dispatch({ type: "FOCUS_TAB", id: existing.id });
        return;
      }
      const content = await readTextFile(path);
      dispatch({
        type: "OPEN_TAB",
        tab: {
          id: path,
          filePath: path,
          title: name ?? basenameOf(path),
          content,
          isDirty: false,
          mode: "rich",
        },
      });
    },
    [state.tabs],
  );

  const addFolder = useCallback(async () => {
    const picked = await openFolderDialog();
    if (!picked) return null;
    const node = await readProjectNode(picked);
    dispatch({ type: "ADD_ROOT", rootPath: picked, node });
    return picked;
  }, []);

  const openFilePicker = useCallback(async () => {
    const picked = await openFileDialog();
    if (!picked) return null;
    await openFile(picked);
    return picked;
  }, [openFile]);

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
      openFile,
      addFolder,
      openFilePicker,
      isHydrated,
      defaultFolder,
      setDefaultFolder,
      isAutosaveEnabled,
      setIsAutosaveEnabled,
    }),
    [
      state,
      activeTab,
      refreshTree,
      openFile,
      addFolder,
      openFilePicker,
      isHydrated,
      defaultFolder,
      isAutosaveEnabled,
    ],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace() {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used within a WorkspaceProvider");
  return ctx;
}
