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
