# Workspace Session Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the application's workspace and UI state (opened folder, open tabs, active tab, editor modes, sidebar collapsed state) across app restarts.

**Architecture:** Save lightweight workspace session metadata (`rootPath`, tab file paths, tab modes, tab order, `activeFilePath`, `isSidebarCollapsed`) to `localStorage` under key `zyplus:workspace-session`. On application startup, load and validate the session, rebuild the file tree from disk, reload open tab contents fresh from disk, and restore the UI state seamlessly while gracefully skipping any files that no longer exist.

**Tech Stack:** React 19, TypeScript, Bun, Tauri v2 plugins (`@tauri-apps/plugin-fs`, `@tauri-apps/api/path`), `localStorage`.

## Global Constraints

- Storage key: `zyplus:workspace-session`
- Schema version: `1`
- Content loading: Fresh from disk upon restore; missing files are skipped without runtime errors
- Package manager and runtime: `bun`
- Browser verification: Use `agent-browser` for headless verification

---

### Task 1: Create Session Storage Library with Types and Unit Tests

**Files:**
- Create: `src/lib/sessionStorage.ts`
- Create: `tests/sessionStorage.test.ts`

**Interfaces:**
- Produces:
  ```typescript
  export interface PersistedTab {
    filePath: string;
    mode: "rich" | "plain";
  }

  export interface PersistedWorkspaceSession {
    version: 1;
    rootPath: string | null;
    tabs: PersistedTab[];
    activeFilePath: string | null;
    isSidebarCollapsed: boolean;
  }

  export function loadSession(): PersistedWorkspaceSession | null;
  export function saveSession(session: PersistedWorkspaceSession): void;
  export function clearSession(): void;
  ```

- [ ] **Step 1: Write the failing test**

```typescript
// tests/sessionStorage.test.ts
import { describe, it, expect, beforeEach } from "bun:test";
import {
  loadSession,
  saveSession,
  clearSession,
  type PersistedWorkspaceSession,
} from "../src/lib/sessionStorage";

describe("sessionStorage", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("returns null when no session is stored", () => {
    expect(loadSession()).toBeNull();
  });

  it("saves and loads a valid session", () => {
    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/path/to/project",
      tabs: [
        { filePath: "/path/to/project/note1.md", mode: "rich" },
        { filePath: "/path/to/project/note2.md", mode: "plain" },
      ],
      activeFilePath: "/path/to/project/note2.md",
      isSidebarCollapsed: true,
    };

    saveSession(session);
    const loaded = loadSession();
    expect(loaded).toEqual(session);
  });

  it("handles corrupted JSON gracefully and returns null", () => {
    localStorage.setItem("zyplus:workspace-session", "{ invalid json ");
    expect(loadSession()).toBeNull();
  });

  it("handles invalid schema version gracefully and returns null", () => {
    localStorage.setItem(
      "zyplus:workspace-session",
      JSON.stringify({ version: 99, rootPath: "/invalid" }),
    );
    expect(loadSession()).toBeNull();
  });

  it("clears stored session", () => {
    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/path/to/project",
      tabs: [],
      activeFilePath: null,
      isSidebarCollapsed: false,
    };
    saveSession(session);
    clearSession();
    expect(loadSession()).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test tests/sessionStorage.test.ts`
Expected: FAIL with "Cannot find module '../src/lib/sessionStorage'"

- [ ] **Step 3: Write minimal implementation**

```typescript
// src/lib/sessionStorage.ts
export const SESSION_STORAGE_KEY = "zyplus:workspace-session";
export const CURRENT_SESSION_VERSION = 1;

export interface PersistedTab {
  filePath: string;
  mode: "rich" | "plain";
}

export interface PersistedWorkspaceSession {
  version: 1;
  rootPath: string | null;
  tabs: PersistedTab[];
  activeFilePath: string | null;
  isSidebarCollapsed: boolean;
}

export function loadSession(): PersistedWorkspaceSession | null {
  if (typeof window === "undefined" || !window.localStorage) return null;
  try {
    const raw = localStorage.getItem(SESSION_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed || parsed.version !== CURRENT_SESSION_VERSION) return null;
    if (typeof parsed.rootPath !== "string" && parsed.rootPath !== null) return null;
    if (!Array.isArray(parsed.tabs)) return null;

    const validatedTabs: PersistedTab[] = [];
    for (const item of parsed.tabs) {
      if (
        item &&
        typeof item.filePath === "string" &&
        (item.mode === "rich" || item.mode === "plain")
      ) {
        validatedTabs.push({ filePath: item.filePath, mode: item.mode });
      }
    }

    return {
      version: CURRENT_SESSION_VERSION,
      rootPath: parsed.rootPath,
      tabs: validatedTabs,
      activeFilePath: typeof parsed.activeFilePath === "string" ? parsed.activeFilePath : null,
      isSidebarCollapsed: Boolean(parsed.isSidebarCollapsed),
    };
  } catch {
    return null;
  }
}

export function saveSession(session: PersistedWorkspaceSession): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    localStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify(session));
  } catch (err) {
    console.warn("Failed to persist workspace session to localStorage:", err);
  }
}

export function clearSession(): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    localStorage.removeItem(SESSION_STORAGE_KEY);
  } catch {
    // Ignore clear error
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test tests/sessionStorage.test.ts`
Expected: PASS (all 5 tests passing)

- [ ] **Step 5: Commit**

```bash
git add src/lib/sessionStorage.ts tests/sessionStorage.test.ts
git commit -m "feat(storage): create session storage library with schema validation"
```

---

### Task 2: Add RESTORE_WORKSPACE Action to Workspace Reducer

**Files:**
- Modify: `src/state/workspaceStore.tsx`
- Create: `tests/workspaceStore.test.ts`

**Interfaces:**
- Consumes: `TabState`, `TreeNode`, `WorkspaceState` from `src/state/workspaceStore.tsx`
- Produces: `RESTORE_WORKSPACE` action type in `Action`:
  ```typescript
  | {
      type: "RESTORE_WORKSPACE";
      rootPath: string | null;
      tree: TreeNode[];
      tabs: TabState[];
      activeTabId: string | null;
    }
  ```

- [ ] **Step 1: Write the failing test**

```typescript
// tests/workspaceStore.test.ts
import { describe, it, expect } from "bun:test";
import {
  workspaceReducer,
  type WorkspaceState,
  type TabState,
} from "../src/state/workspaceStore";

describe("workspaceReducer", () => {
  const initial: WorkspaceState = {
    rootPath: null,
    tree: [],
    tabs: [],
    activeTabId: null,
  };

  it("handles RESTORE_WORKSPACE properly", () => {
    const tabs: TabState[] = [
      {
        id: "/project/doc1.md",
        filePath: "/project/doc1.md",
        title: "doc1.md",
        content: "# Doc 1",
        isDirty: false,
        mode: "rich",
      },
      {
        id: "/project/doc2.md",
        filePath: "/project/doc2.md",
        title: "doc2.md",
        content: "Plain Doc",
        isDirty: false,
        mode: "plain",
      },
    ];

    const next = workspaceReducer(initial, {
      type: "RESTORE_WORKSPACE",
      rootPath: "/project",
      tree: [{ id: "/project/doc1.md", name: "doc1.md", isFolder: false }],
      tabs,
      activeTabId: "/project/doc2.md",
    });

    expect(next.rootPath).toBe("/project");
    expect(next.tree.length).toBe(1);
    expect(next.tabs.length).toBe(2);
    expect(next.activeTabId).toBe("/project/doc2.md");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test tests/workspaceStore.test.ts`
Expected: FAIL with "workspaceReducer is not exported" or "Unknown action type"

- [ ] **Step 3: Modify workspaceStore.tsx**

Export `workspaceReducer` and add `RESTORE_WORKSPACE` case:

```typescript
// In src/state/workspaceStore.tsx
export type Action =
  | { type: "OPEN_ROOT"; rootPath: string; tree: TreeNode[] }
  | { type: "SET_TREE"; tree: TreeNode[] }
  | { type: "OPEN_TAB"; tab: TabState }
  | { type: "FOCUS_TAB"; id: string }
  | { type: "CLOSE_TAB"; id: string }
  | { type: "REORDER_TAB"; id: string; targetId: string | null }
  | { type: "UPDATE_TAB_CONTENT"; id: string; content: string }
  | { type: "SET_TAB_MODE"; id: string; mode: TabMode }
  | { type: "SAVE_TAB_SUCCESS"; id: string }
  | { type: "REMAP_TAB_PATHS"; oldPrefix: string; newPrefix: string }
  | { type: "CLOSE_TABS_UNDER"; prefix: string }
  | {
      type: "RESTORE_WORKSPACE";
      rootPath: string | null;
      tree: TreeNode[];
      tabs: TabState[];
      activeTabId: string | null;
    };

export function workspaceReducer(state: WorkspaceState, action: Action): WorkspaceState {
  switch (action.type) {
    case "RESTORE_WORKSPACE":
      return {
        rootPath: action.rootPath,
        tree: action.tree,
        tabs: action.tabs,
        activeTabId: action.activeTabId,
      };
    // ... existing cases ...
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test tests/workspaceStore.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/state/workspaceStore.tsx tests/workspaceStore.test.ts
git commit -m "feat(store): export workspaceReducer and add RESTORE_WORKSPACE action"
```

---

### Task 3: Implement Startup Hydration & Automatic Session Persistence

**Files:**
- Modify: `src/state/workspaceStore.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `loadSession`, `saveSession` from `src/lib/sessionStorage.ts`
- Consumes: `readDirRecursive`, `readTextFile` from `src/lib/fs.ts`
- Produces: Automatic startup session restoration and persistence across edits, tabs, and sidebar toggling.

- [ ] **Step 1: Update WorkspaceProvider in `src/state/workspaceStore.tsx`**

1. Import `loadSession`, `saveSession`, `PersistedWorkspaceSession`, and `basenameOf`.
2. Add an `isHydrated` state flag (`useRef(false)` or `useState(false)`) to prevent overwriting the saved session before initial hydration completes.
3. Add a startup `useEffect`:
   - Calls `loadSession()`.
   - If a stored session exists with `rootPath`:
     - Tries `await readDirRecursive(stored.rootPath)`.
     - Iterates through `stored.tabs`: for each tab, tries `await readTextFile(tab.filePath)`. If successful, creates `TabState`.
     - Validates `activeTabId`: if `stored.activeFilePath` matches one of the restored tabs, sets `activeTabId = stored.activeFilePath`; otherwise picks the last valid tab or `null`.
     - Dispatches `RESTORE_WORKSPACE`.
   - Marks `isHydrated.current = true`.
4. Add a persistence `useEffect`:
   - Only runs if `isHydrated.current === true`.
   - Reads `state.rootPath`, `state.tabs`, `state.activeTabId`.
   - Maps `state.tabs` to `{ filePath: t.filePath, mode: t.mode }`.
   - Gets existing stored session (to preserve `isSidebarCollapsed` if needed) and calls `saveSession(...)`.

- [ ] **Step 2: Update `src/App.tsx` for Sidebar State Persistence**

1. Initialize `isSidebarCollapsed`:
   ```typescript
   const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(() => {
     return loadSession()?.isSidebarCollapsed ?? false;
   });
   ```
2. When `isSidebarCollapsed` changes, sync with stored session via `saveSession`:
   ```typescript
   useEffect(() => {
     const current = loadSession();
     if (current) {
       saveSession({ ...current, isSidebarCollapsed });
     }
   }, [isSidebarCollapsed]);
   ```

- [ ] **Step 3: Run project build and tests**

Run: `bun test && bun run build`
Expected: All tests pass and build succeeds with exit code 0.

- [ ] **Step 4: Commit**

```bash
git add src/state/workspaceStore.tsx src/App.tsx
git commit -m "feat(workspace): restore workspace state on startup and persist changes to session storage"
```

---

### Task 4: End-to-End Verification with Agent Browser

**Files:**
- Create scratch test workspace directory or files if needed for testing
- Run Vite dev server and launch headless browser via `agent-browser`

- [ ] **Step 1: Start dev server in background**

Run: `bun run dev --port 5173`
Check that http://localhost:5173 loads.

- [ ] **Step 2: Automate verification using agent-browser**

1. Navigate to `http://localhost:5173`.
2. Seed mock session into `localStorage` using browser eval:
   ```javascript
   localStorage.setItem('zyplus:workspace-session', JSON.stringify({
     version: 1,
     rootPath: '/Users/fankrits/dev/Zyplus-editor',
     tabs: [
       { filePath: '/Users/fankrits/dev/Zyplus-editor/README.md', mode: 'plain' }
     ],
     activeFilePath: '/Users/fankrits/dev/Zyplus-editor/README.md',
     isSidebarCollapsed: true
   }));
   ```
3. Reload browser page via `agent_browser_reload`.
4. Inspect DOM snapshot to verify:
   - Sidebar is collapsed (toggle button visible, sidebar panel hidden).
   - Tab "README.md" is restored and active.
   - CodeMirror plain text editor is rendered with README.md contents.
5. Click sidebar toggle to expand sidebar:
   - Verify sidebar opens with root folder files listed.
6. Check `localStorage.getItem('zyplus:workspace-session')` via eval:
   - Verify `isSidebarCollapsed` updated to `false`.

- [ ] **Step 3: Stop background dev server and cleanup**

Ensure all background tasks are closed and clean.

- [ ] **Step 4: Commit**

```bash
git commit --allow-empty -m "test: verify workspace session restoration across reloads"
```
