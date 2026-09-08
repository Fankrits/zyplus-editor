import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  basenameOf,
  openFileDialog,
  openFolderDialog,
  readProjectNode,
  readTextFile,
  writeTextFile,
} from "../lib/fs";

import { loadSession, saveSession, type PersistedWorkspaceSession } from "../lib/sessionStorage";
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
export { workspaceReducer, restoreWorkspaceFromSession } from "./workspaceReducer";

/**
 * The store is split into three subscriptions because typing rewrites `state.tabs`
 * on every keystroke. A single context would re-render the file tree, the sidebar
 * and the app shell along with it; this way only the components that actually show
 * tab state do. Actions are a fourth, permanently stable object — callbacks read
 * live state through `getState()` instead of closing over it, so the consumers that
 * only dispatch never re-render at all.
 */

/** Project roots and their file trees. Changes only when the filesystem does. */
interface WorkspaceTreeValue {
  roots: string[];
  tree: TreeNode[];
}

/** Open documents. Changes on every keystroke. */
interface WorkspaceTabsValue {
  tabs: TabState[];
  activeTabId: string | null;
  activeTab: TabState | null;
}

/** Session-level settings. `activeTabId` is here too so consumers that only need
 *  to know which file is focused don't have to subscribe to tab content. */
interface WorkspaceSessionValue {
  /** False until the stored session has been read back. */
  isHydrated: boolean;
  /** Folder created on first run; where the header's "new file" lands. */
  defaultFolder: string | null;
  /** Save dirty tabs automatically shortly after typing stops. */
  isAutosaveEnabled: boolean;
  isSidebarCollapsed: boolean;
  activeTabId: string | null;
  hasTabs: boolean;
}

interface WorkspaceActionsValue {
  dispatch: React.Dispatch<Action>;
  /** Current state, for event handlers that would otherwise close over it. */
  getState: () => WorkspaceState;
  refreshTree: () => Promise<void>;
  /** Opens a file in a tab, focusing it if already open. Returns true if opened successfully. */
  openFile: (path: string, name?: string) => Promise<boolean>;
  /** Prompts for a folder or adds the specified directory as a project root. Returns the path. */
  addFolder: (dir?: string) => Promise<string | null>;
  /** Prompts for a file and opens it in a tab. Returns the picked path. */
  openFilePicker: () => Promise<string | null>;

  setDefaultFolder: (path: string) => void;
  setIsAutosaveEnabled: (enabled: boolean) => void;
  setIsSidebarCollapsed: (collapsed: boolean | ((prev: boolean) => boolean)) => void;
}

const TreeContext = createContext<WorkspaceTreeValue | null>(null);
const TabsContext = createContext<WorkspaceTabsValue | null>(null);
const SessionContext = createContext<WorkspaceSessionValue | null>(null);
const ActionsContext = createContext<WorkspaceActionsValue | null>(null);

function useRequired<T>(ctx: React.Context<T | null>): T {
  const value = useContext(ctx);
  if (!value) throw new Error("useWorkspace must be used within a WorkspaceProvider");
  return value;
}

export const useWorkspaceTree = () => useRequired(TreeContext);
export const useWorkspaceTabs = () => useRequired(TabsContext);
export const useWorkspaceSession = () => useRequired(SessionContext);
export const useWorkspaceActions = () => useRequired(ActionsContext);

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(workspaceReducer, initialState);
  const [isHydrated, setIsHydrated] = useState(false);
  const [defaultFolder, setDefaultFolder] = useState<string | null>(null);
  const [isAutosaveEnabled, setIsAutosaveEnabled] = useState(false);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(
    () => loadSession()?.isSidebarCollapsed ?? false,
  );

  const stateRef = useRef(state);
  stateRef.current = state;
  const getState = useCallback(() => stateRef.current, []);

  useEffect(() => {
    let isMounted = true;
    async function hydrate() {
      const stored = loadSession();
      if (stored?.defaultFolder) setDefaultFolder(stored.defaultFolder);
      if (stored?.isAutosaveEnabled) setIsAutosaveEnabled(true);
      if (stored && (stored.roots.length > 0 || stored.tabs.length > 0)) {
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

  // localStorage writes are synchronous, and nothing here changes while typing —
  // the session records paths and modes, not content — so skip the identical write.
  const lastPersisted = useRef<string | null>(null);
  useEffect(() => {
    if (!isHydrated) return;
    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: state.roots,
      defaultFolder,
      tabs: state.tabs.map((t) => ({ filePath: t.filePath, mode: t.mode })),
      activeFilePath: state.activeTabId,
      isSidebarCollapsed,
      isAutosaveEnabled,
    };
    const json = JSON.stringify(session);
    if (json === lastPersisted.current) return;
    lastPersisted.current = json;
    saveSession(session);
  }, [
    isHydrated,
    state.roots,
    state.tabs,
    state.activeTabId,
    defaultFolder,
    isAutosaveEnabled,
    isSidebarCollapsed,
  ]);

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
    const { roots } = stateRef.current;
    if (roots.length === 0) return;
    const tree = await Promise.all(roots.map(readProjectNode));
    dispatch({ type: "SET_TREE", tree });
  }, []);

  const openFile = useCallback(async (path: string, name?: string): Promise<boolean> => {
    const { tabs } = getState();
    const existing = tabs.find((t) => t.filePath === path);
    if (existing) {
      dispatch({ type: "FOCUS_TAB", id: existing.id });
      return true;
    }
    try {
      const content = await readTextFile(path);
      dispatch({
        type: "OPEN_TAB",
        tab: {
          id: path,
          filePath: path,
          title: name ?? basenameOf(path),
          content,
          savedContent: content,
          isDirty: false,
          mode: "rich",
        },
      });
      return true;
    } catch (err) {
      console.error(`Failed to open file at "${path}":`, err);
      return false;
    }
  }, []);


  const addFolder = useCallback(async (dir?: string) => {
    const picked = dir ?? (await openFolderDialog());
    if (!picked) return null;
    try {
      const node = await readProjectNode(picked);
      dispatch({ type: "ADD_ROOT", rootPath: picked, node });
      return picked;
    } catch (err) {
      console.error(`Failed to open folder at "${picked}":`, err);
      return null;
    }
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

  const treeValue = useMemo(
    () => ({ roots: state.roots, tree: state.tree }),
    [state.roots, state.tree],
  );

  const tabsValue = useMemo(
    () => ({ tabs: state.tabs, activeTabId: state.activeTabId, activeTab }),
    [state.tabs, state.activeTabId, activeTab],
  );

  const hasTabs = state.tabs.length > 0;
  const sessionValue = useMemo(
    () => ({
      isHydrated,
      defaultFolder,
      isAutosaveEnabled,
      isSidebarCollapsed,
      activeTabId: state.activeTabId,
      hasTabs,
    }),
    [isHydrated, defaultFolder, isAutosaveEnabled, isSidebarCollapsed, state.activeTabId, hasTabs],
  );

  const actionsValue = useMemo(
    () => ({
      dispatch,
      getState,
      refreshTree,
      openFile,
      addFolder,
      openFilePicker,
      setDefaultFolder,
      setIsAutosaveEnabled,
      setIsSidebarCollapsed,
    }),
    [getState, refreshTree, openFile, addFolder, openFilePicker],
  );

  return (
    <ActionsContext.Provider value={actionsValue}>
      <SessionContext.Provider value={sessionValue}>
        <TreeContext.Provider value={treeValue}>
          <TabsContext.Provider value={tabsValue}>{children}</TabsContext.Provider>
        </TreeContext.Provider>
      </SessionContext.Provider>
    </ActionsContext.Provider>
  );
}

/**
 * Everything at once. Convenient, but it subscribes to all four contexts — prefer
 * the narrow hooks in components that render while the user is typing.
 */
export function useWorkspace() {
  const tree = useWorkspaceTree();
  const tabs = useWorkspaceTabs();
  const session = useWorkspaceSession();
  const actions = useWorkspaceActions();
  return useMemo(
    () => ({
      ...actions,
      ...session,
      activeTab: tabs.activeTab,
      state: {
        roots: tree.roots,
        tree: tree.tree,
        tabs: tabs.tabs,
        activeTabId: tabs.activeTabId,
      } as WorkspaceState,
    }),
    [tree, tabs, session, actions],
  );
}
