# Responsive Design Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a responsive layout across Zyplus Editor using HeroUI's compound `Drawer` component on mobile viewports (<768px), adaptive tab bars and editor padding, while preserving the desktop multi-column collapsible layout.

**Architecture:** A viewport media query hook (`useMediaQuery` / `useIsDesktop`) drives viewport mode. On desktop (`≥768px`), the existing persistent collapsible inline sidebar is rendered. On mobile (`<768px`), the inline sidebar is hidden and the sidebar toggle button activates an off-canvas HeroUI `Drawer` (`placement="left"`) that auto-closes on file selection. Editor padding and tab mode buttons adapt to compact layouts.

**Tech Stack:** React 19, TypeScript, Tailwind CSS v4, HeroUI v3 (`@heroui/react`), Bun test runner, Vite, Agent-Browser.

## Global Constraints
- Reuse HeroUI v3 components (`Drawer`, `Drawer.Backdrop`, `Drawer.Content`, `Drawer.Dialog`, etc.) and avoid custom drawer implementations.
- Desktop sidebar collapse state persists in session storage; mobile drawer state is transient.
- Code changes must build cleanly with `bun run build` and pass all unit tests with `bun test`.
- Each task ends with an isolated git commit.

---

### Task 1: Viewport Detection Hook (`useMediaQuery` & `useIsDesktop`)

**Files:**
- Create: `src/lib/useMediaQuery.ts`
- Test: `tests/useMediaQuery.test.ts`

**Interfaces:**
- Produces:
  - `useMediaQuery(query: string): boolean`
  - `useIsDesktop(): boolean`

- [ ] **Step 1: Write failing unit test for `useMediaQuery` and `useIsDesktop`**

Create `tests/useMediaQuery.test.ts`:
```typescript
import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { renderHook, act } from "@testing-library/react"; // or pure js test for matchMedia listener logic
import { useMediaQuery, useIsDesktop } from "../src/lib/useMediaQuery";

describe("useMediaQuery & useIsDesktop", () => {
  let listeners: ((e: MediaQueryListEvent) => void)[] = [];
  let matchesValue = false;

  beforeEach(() => {
    listeners = [];
    matchesValue = false;
    window.matchMedia = (query: string): MediaQueryList => ({
      matches: query === "(min-width: 768px)" ? matchesValue : false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
        listeners.push(handler);
      },
      removeEventListener: (_event: string, handler: (e: MediaQueryListEvent) => void) => {
        listeners = listeners.filter((h) => h !== handler);
      },
      dispatchEvent: () => true,
    } as unknown as MediaQueryList);
  });

  it("evaluates initial match value", () => {
    matchesValue = true;
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(true);
  });

  it("updates when media query changes", () => {
    matchesValue = false;
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(false);

    act(() => {
      matchesValue = true;
      listeners.forEach((l) => l({ matches: true } as MediaQueryListEvent));
    });

    expect(result.current).toBe(true);
  });
});
```

- [ ] **Step 2: Run test to verify failure**

Run: `bun test tests/useMediaQuery.test.ts`
Expected: FAIL (module not found)

- [ ] **Step 3: Implement `src/lib/useMediaQuery.ts`**

Create `src/lib/useMediaQuery.ts`:
```typescript
import { useEffect, useState } from "react";

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(() => {
    if (typeof window === "undefined" || !window.matchMedia) return false;
    return window.matchMedia(query).matches;
  });

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mql = window.matchMedia(query);
    const handler = (e: MediaQueryListEvent) => setMatches(e.matches);
    setMatches(mql.matches);
    mql.addEventListener("change", handler);
    return () => mql.removeEventListener("change", handler);
  }, [query]);

  return matches;
}

export function useIsDesktop(): boolean {
  return useMediaQuery("(min-width: 768px)");
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test tests/useMediaQuery.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/lib/useMediaQuery.ts tests/useMediaQuery.test.ts
git commit -m "feat(responsive): add useMediaQuery and useIsDesktop hooks with unit tests"
```

---

### Task 2: Refactor Sidebar into Reusable Component & Integrate HeroUI Mobile Drawer

**Files:**
- Modify: `src/components/Sidebar/Sidebar.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes:
  - `useIsDesktop` from `src/lib/useMediaQuery`
  - `Drawer` compound components from `@heroui/react`
- Produces:
  - `SidebarContent`: Reusable file tree and folder header
  - `AppShell`: Dual-mode rendering (desktop inline sidebar vs mobile off-canvas `Drawer`)

- [ ] **Step 1: Update `src/components/Sidebar/Sidebar.tsx`**

Export `SidebarContent` so both inline desktop sidebar and mobile drawer can share the exact same UI and handlers without duplication:
```typescript
export interface SidebarContentProps {
  onRequestCreate: (kind: "file" | "folder", targetDir: string) => void;
  onOpenFile?: (path: string, name: string) => void;
}

export function SidebarContent({ onRequestCreate, onOpenFile }: SidebarContentProps) {
  const { state, dispatch } = useWorkspace();

  const handleOpenFolder = useCallback(async () => {
    const picked = await fs.openFolderDialog();
    if (!picked) return;
    const tree = await fs.readDirRecursive(picked);
    dispatch({ type: "OPEN_ROOT", rootPath: picked, tree });
  }, [dispatch]);

  const defaultOpenFile = useCallback(
    async (path: string, name: string) => {
      const existing = state.tabs.find((t) => t.filePath === path);
      if (existing) {
        dispatch({ type: "FOCUS_TAB", id: existing.id });
        return;
      }
      const content = await fs.readTextFile(path);
      dispatch({
        type: "OPEN_TAB",
        tab: { id: path, filePath: path, title: name, content, isDirty: false, mode: "rich" },
      });
    },
    [state.tabs, dispatch],
  );

  const handleOpenFile = onOpenFile ?? defaultOpenFile;

  if (!state.rootPath) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-4">
        <Button variant="primary" onPress={handleOpenFolder}>
          <HugeiconsIcon icon={FolderOpenIcon} size={18} />
          Open Folder
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex shrink-0 items-center justify-between gap-1 px-2 py-2">
        <span className="truncate text-xs font-medium uppercase tracking-wide text-neutral-500">
          {state.rootPath.split(/[\\/]/).pop()}
        </span>
        <div className="flex shrink-0 gap-1">
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="New file"
            onPress={() => onRequestCreate("file", state.rootPath!)}
          >
            <HugeiconsIcon icon={FileAddIcon} size={16} />
          </Button>
          <Button
            size="sm"
            isIconOnly
            variant="ghost"
            aria-label="New folder"
            onPress={() => onRequestCreate("folder", state.rootPath!)}
          >
            <HugeiconsIcon icon={FolderAddIcon} size={16} />
          </Button>
        </div>
      </div>
      <FileTree onOpenFile={handleOpenFile} onRequestCreate={onRequestCreate} />
    </div>
  );
}

export function Sidebar({ onRequestCreate, isCollapsed }: { onRequestCreate: (kind: "file" | "folder", targetDir: string) => void; isCollapsed: boolean }) {
  if (isCollapsed) return null;
  return (
    <aside className="hidden md:flex h-full w-64 shrink-0 flex-col border-r border-black/10 dark:border-white/10">
      <SidebarContent onRequestCreate={onRequestCreate} />
    </aside>
  );
}
```

- [ ] **Step 2: Update `src/App.tsx` with HeroUI `Drawer` and Mobile Auto-Close**

Import `Drawer` from `@heroui/react`, `useIsDesktop` from `./lib/useMediaQuery`, and integrate `isMobileDrawerOpen`:
- When opening a file from the drawer on mobile, auto-close drawer: `setIsMobileDrawerOpen(false)`.
- When requesting to create a file or folder from mobile drawer, auto-close drawer: `setIsMobileDrawerOpen(false)`.
- Pass `isDesktop` and `isMobileDrawerOpen` to `TabBar`.

- [ ] **Step 3: Run unit tests and verify build**

Run: `bun test && bun run build`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/components/Sidebar/Sidebar.tsx src/App.tsx
git commit -m "feat(responsive): add HeroUI drawer for mobile with auto-closing file selection"
```

---

### Task 3: TabBar Responsive Adaptations

**Files:**
- Modify: `src/components/Tabs/TabBar.tsx`

**Interfaces:**
- Consumes:
  - `isDesktop: boolean`
  - `isMobileDrawerOpen: boolean`
- Produces:
  - Compact icon-only Rich/Plain mode buttons on mobile viewports
  - Responsive tab title width (`max-w-[8rem] sm:max-w-[14rem]`)
  - Highlighted toggle sidebar button according to active desktop/mobile state

- [ ] **Step 1: Modify `src/components/Tabs/TabBar.tsx`**

- Update `TabItem` title `className`: change `max-w-[14rem]` to `max-w-[8rem] sm:max-w-[14rem]`.
- Update `TabBarProps` to include `isDesktop: boolean` and `isMobileDrawerOpen: boolean`.
- Update toggle sidebar button styling:
  ```tsx
  className={`flex size-8 shrink-0 items-center justify-center rounded-3xl no-highlight outline-none hover:opacity-70 focus-visible:status-focused ${
    (isDesktop ? !isSidebarCollapsed : isMobileDrawerOpen) ? "text-accent" : "text-muted"
  }`}
  ```
- Update mode switch buttons:
  Add `isIconOnly={!isDesktop}` so on small screens only the icon is rendered, freeing valuable toolbar space.

- [ ] **Step 2: Run unit tests and verify build**

Run: `bun test && bun run build`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/components/Tabs/TabBar.tsx
git commit -m "feat(responsive): adapt TabBar with compact mobile buttons and tab title truncation"
```

---

### Task 4: Responsive Editor Padding & Gutters

**Files:**
- Modify: `src/components/Editor/RichTextEditor.tsx`
- Modify: `src/components/Editor/plainTextTheme.ts`

- [ ] **Step 1: Update `src/components/Editor/RichTextEditor.tsx`**

Change root container padding:
From: `className="h-full overflow-y-auto px-8 py-6"`
To: `className="h-full overflow-y-auto px-4 sm:px-8 py-4 sm:py-6"`

- [ ] **Step 2: Update `src/components/Editor/plainTextTheme.ts`**

Change `.cm-content` padding to be responsive using `@media (min-width: 640px)`:
```typescript
  ".cm-content": {
    maxWidth: "48rem",
    margin: "0 auto",
    padding: "1.5rem 1rem 8rem 1rem",
    "@media (min-width: 640px)": {
      padding: "3rem 1.5rem 12rem 1.5rem",
    },
    fontSize: "15px",
    caretColor: "var(--accent, currentColor)",
  },
```

- [ ] **Step 3: Run unit tests and verify build**

Run: `bun test && bun run build`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/components/Editor/RichTextEditor.tsx src/components/Editor/plainTextTheme.ts
git commit -m "feat(responsive): add responsive padding for RichTextEditor and PlainTextEditor"
```

---

### Task 5: End-to-End Verification with Browser Automation

**Files:**
- Test verification script / Agent-browser interaction

- [ ] **Step 1: Run all automated unit tests**

Run: `bun test`
Expected: All tests pass.

- [ ] **Step 2: Run production TypeScript and Vite build**

Run: `bun run build`
Expected: Clean build with exit code 0.

- [ ] **Step 3: Verify responsive viewports using Agent-Browser**

1. Launch Vite dev server via background task: `bun run dev`
2. Using `agent-browser`:
   - Open browser with viewport width 390px (Mobile):
     - Verify inline sidebar is hidden.
     - Verify clicking sidebar button opens HeroUI `Drawer`.
     - Verify selecting a file loads file content and auto-closes drawer.
     - Verify Rich/Plain mode buttons are compact icon-only.
   - Resize / Open browser with viewport width 1280px (Desktop):
     - Verify desktop inline sidebar is visible.
     - Verify sidebar collapse/expand works and persists.
     - Verify mode buttons display full text labels.
3. Take snapshots / screenshots for the walkthrough artifact.

- [ ] **Step 4: Final commit and cleanup**

```bash
git status
```
