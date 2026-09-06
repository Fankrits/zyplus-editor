import { describe, it, expect, beforeEach } from "bun:test";
import {
  loadSession,
  saveSession,
  clearSession,
  type PersistedWorkspaceSession,
} from "../src/lib/sessionStorage";

class LocalStorageMock implements Storage {
  private store: Record<string, string> = {};

  get length(): number {
    return Object.keys(this.store).length;
  }

  clear(): void {
    this.store = {};
  }

  getItem(key: string): string | null {
    return Object.prototype.hasOwnProperty.call(this.store, key) ? this.store[key] : null;
  }

  setItem(key: string, value: string): void {
    this.store[key] = String(value);
  }

  removeItem(key: string): void {
    delete this.store[key];
  }

  key(index: number): string | null {
    const keys = Object.keys(this.store);
    return keys[index] ?? null;
  }
}

if (typeof globalThis.localStorage === "undefined") {
  const storage = new LocalStorageMock();
  globalThis.localStorage = storage;
  (globalThis as any).window = globalThis;
}

describe("sessionStorage", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("returns null when no session is stored", () => {
    expect(loadSession()).toBeNull();
  });

  it("saves and loads a valid session", () => {
    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: ["/path/to/project", "/other/project"],
      defaultFolder: "/path/to/project",
      tabs: [
        { filePath: "/path/to/project/note1.md", mode: "rich" },
        { filePath: "/path/to/project/note2.md", mode: "plain" },
      ],
      activeFilePath: "/path/to/project/note2.md",
      isSidebarCollapsed: true,
      isAutosaveEnabled: true,
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
      version: 2,
      roots: ["/path/to/project"],
      defaultFolder: null,
      tabs: [],
      activeFilePath: null,
      isSidebarCollapsed: false,
    };
    saveSession(session);
    clearSession();
    expect(loadSession()).toBeNull();
  });

  it("filters out invalid tab entries", () => {
    localStorage.setItem(
      "zyplus:workspace-session",
      JSON.stringify({
        version: 2,
        roots: ["/path/to/project"],
        tabs: [
          { filePath: "/valid.md", mode: "rich" },
          { filePath: 123, mode: "rich" },
          { filePath: "/bad-mode.md", mode: "unknown" },
          null,
          { filePath: "/valid2.md", mode: "plain" },
        ],
        activeFilePath: "/valid.md",
        isSidebarCollapsed: false,
      }),
    );

    const loaded = loadSession();
    expect(loaded).not.toBeNull();
    expect(loaded?.tabs).toEqual([
      { filePath: "/valid.md", mode: "rich" },
      { filePath: "/valid2.md", mode: "plain" },
    ]);
  });

  it("migrates a v1 session into a single project root", () => {
    localStorage.setItem(
      "zyplus:workspace-session",
      JSON.stringify({
        version: 1,
        rootPath: "/path/to/project",
        tabs: [{ filePath: "/path/to/project/note.md", mode: "rich" }],
        activeFilePath: "/path/to/project/note.md",
        isSidebarCollapsed: false,
      }),
    );
    const loaded = loadSession();
    expect(loaded?.version).toBe(2);
    expect(loaded?.roots).toEqual(["/path/to/project"]);
    expect(loaded?.defaultFolder).toBeNull();
    expect(loaded?.tabs).toHaveLength(1);
  });

  it("handles non-string/non-null rootPath by returning null", () => {
    localStorage.setItem(
      "zyplus:workspace-session",
      JSON.stringify({
        version: 1,
        rootPath: 12345,
        tabs: [],
        activeFilePath: null,
        isSidebarCollapsed: false,
      }),
    );
    expect(loadSession()).toBeNull();
  });

  it("handles non-array tabs by returning null", () => {
    localStorage.setItem(
      "zyplus:workspace-session",
      JSON.stringify({
        version: 2,
        roots: ["/path/to/project"],
        tabs: "not-an-array",
        activeFilePath: null,
        isSidebarCollapsed: false,
      }),
    );
    expect(loadSession()).toBeNull();
  });
});
