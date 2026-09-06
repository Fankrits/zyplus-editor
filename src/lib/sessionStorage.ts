export const SESSION_STORAGE_KEY = "zyplus:workspace-session";
export const CURRENT_SESSION_VERSION = 2;

export interface PersistedTab {
  filePath: string;
  mode: "rich" | "plain";
}

export interface PersistedWorkspaceSession {
  version: 2;
  /** Open project folders, in sidebar order. */
  roots: string[];
  /** Folder the app creates on first run and uses as the default place for new files. */
  defaultFolder: string | null;
  tabs: PersistedTab[];
  activeFilePath: string | null;
  isSidebarCollapsed: boolean;
  /** Write dirty tabs to disk shortly after typing stops. */
  isAutosaveEnabled: boolean;
}

export function loadSession(): PersistedWorkspaceSession | null {
  if (typeof window === "undefined" || !window.localStorage) return null;
  try {
    const raw = localStorage.getItem(SESSION_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed) return null;

    // v1 stored a single `rootPath`; carry it over as the first project.
    let roots: unknown;
    if (parsed.version === 1) {
      if (typeof parsed.rootPath !== "string" && parsed.rootPath !== null) return null;
      roots = parsed.rootPath ? [parsed.rootPath] : [];
    } else if (parsed.version === CURRENT_SESSION_VERSION) {
      roots = parsed.roots;
    } else {
      return null;
    }
    if (!Array.isArray(roots) || roots.some((r) => typeof r !== "string")) return null;
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
      roots: roots as string[],
      defaultFolder: typeof parsed.defaultFolder === "string" ? parsed.defaultFolder : null,
      tabs: validatedTabs,
      activeFilePath: typeof parsed.activeFilePath === "string" ? parsed.activeFilePath : null,
      isSidebarCollapsed: Boolean(parsed.isSidebarCollapsed),
      isAutosaveEnabled: Boolean(parsed.isAutosaveEnabled),
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
