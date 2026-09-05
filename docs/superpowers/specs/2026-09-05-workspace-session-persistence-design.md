# Workspace Session Persistence Design

## Goal
Restore the application's visual and functional state across restarts, matching exactly how the user left it in their latest session: opened folder, open tabs (in order), active tab, editor modes (rich vs plain text), and sidebar collapsed state.

## Architecture & Data Storage

### Storage Mechanism
- Storage medium: Browser `localStorage`.
- Storage key: `zyplus:workspace-session`.
- Format: JSON.

### Data Schema
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
```

Only configuration and paths are persisted—not raw document content or unsaved buffer states. Document contents are read fresh from disk upon restoration.

## Modules & Flow

### 1. `src/lib/sessionStorage.ts`
Provides typed helper functions to read and write the persisted session:
- `loadSession(): PersistedWorkspaceSession | null`: Reads from `localStorage`, validates structure and version, handling missing keys or parse errors safely.
- `saveSession(session: PersistedWorkspaceSession): void`: Writes JSON string to `localStorage`.
- `clearSession(): void`: Clears the stored session (useful for resetting).

### 2. Hydration Flow (`WorkspaceProvider` & `AppShell`)
1. **Initial Mount**:
   - `WorkspaceProvider` loads the persisted session on startup.
   - If `rootPath` is present:
     - Asynchronously verifies the directory exists and loads the directory tree via `fs.readDirRecursive(rootPath)`.
     - If the directory cannot be read (deleted or moved), resets `rootPath` to `null` and skips tabs.
     - For each persisted tab in `tabs`:
       - Asynchronously reads file content via `fs.readTextFile(tab.filePath)`.
       - If read succeeds, adds to restored tab list with `title: basename(tab.filePath)`, `content`, `isDirty: false`, and `mode: tab.mode`.
       - If read fails (file deleted or moved), gracefully ignores the tab.
     - Sets `activeTabId` to `activeFilePath` if that tab was successfully restored; otherwise defaults to the last restored tab or `null`.
     - Dispatches a single hydrated action (`RESTORE_WORKSPACE`) so that the UI updates cleanly without jarring layout shifts.
2. **Sidebar State**:
   - `isSidebarCollapsed` in `AppShell` initializes from `loadSession()?.isSidebarCollapsed ?? false`.

### 3. State Synchronization Flow
- Whenever workspace state changes (`rootPath`, `tabs`, `activeTabId`) or `isSidebarCollapsed` changes:
  - Save the updated session to `localStorage`.
  - Stored tabs preserve their current order in the tab bar.

## Error Handling & Edge Cases
- **Missing / Deleted Files**: If an open tab's file was deleted externally while the app was closed, the file read will reject. The loader catches this and excludes that tab from the restored list.
- **Missing / Deleted Folder**: If the root folder was deleted or moved, `fs.readDirRecursive` fails; the session resets `rootPath` to `null` and tabs to empty.
- **Corrupted Storage**: If `localStorage` has corrupted or invalid JSON, `loadSession()` catches the error, discards the corrupted entry, and returns `null`.
- **Race Conditions**: Hydration happens once on startup before user interaction begins.

## Verification Plan
1. **Unit/Integration Verification**:
   - Verify `loadSession` and `saveSession` correctly serialize and deserialize data, handling invalid JSON and corrupt formats gracefully.
2. **Browser / E2E Verification with Agent Browser**:
   - Run the app via Vite dev server.
   - Open a folder with multiple Markdown files.
   - Open 2+ tabs, toggle one tab to "Plain text" mode.
   - Switch active tab.
   - Collapse the sidebar.
   - Trigger a page reload / simulated restart.
   - Verify:
     - Sidebar remains collapsed.
     - Root folder remains loaded with the file tree visible upon expanding.
     - All tabs are restored with identical file titles and order.
     - Plain text mode is preserved on the designated tab, and rich text on the other.
     - The active tab matches the one selected before reload.
