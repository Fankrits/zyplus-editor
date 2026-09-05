import { describe, it, expect } from "bun:test";
import {
  workspaceReducer,
  basenameOf,
  restoreWorkspaceFromSession,
  type WorkspaceState,
  type TabState,
  type TreeNode,
} from "../src/state/workspaceStore";
import type { PersistedWorkspaceSession } from "../src/lib/sessionStorage";

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

  it("handles RESTORE_WORKSPACE with null rootPath and empty state", () => {
    const populated: WorkspaceState = {
      rootPath: "/previous",
      tree: [{ id: "/previous/doc.md", name: "doc.md", isFolder: false }],
      tabs: [
        {
          id: "/previous/doc.md",
          filePath: "/previous/doc.md",
          title: "doc.md",
          content: "content",
          isDirty: true,
          mode: "rich",
        },
      ],
      activeTabId: "/previous/doc.md",
    };

    const next = workspaceReducer(populated, {
      type: "RESTORE_WORKSPACE",
      rootPath: null,
      tree: [],
      tabs: [],
      activeTabId: null,
    });

    expect(next.rootPath).toBeNull();
    expect(next.tree).toEqual([]);
    expect(next.tabs).toEqual([]);
    expect(next.activeTabId).toBeNull();
  });
});

describe("basenameOf", () => {
  it("extracts filename from posix and windows paths", () => {
    expect(basenameOf("/path/to/file.md")).toBe("file.md");
    expect(basenameOf("C:\\Users\\dev\\notes.txt")).toBe("notes.txt");
    expect(basenameOf("standalone.md")).toBe("standalone.md");
  });
});

describe("restoreWorkspaceFromSession", () => {
  const fakeTree: TreeNode[] = [
    { id: "/test/doc1.md", name: "doc1.md", isFolder: false },
    { id: "/test/doc2.md", name: "doc2.md", isFolder: false },
  ];

  it("returns null when session is null or rootPath is null", async () => {
    expect(await restoreWorkspaceFromSession(null)).toBeNull();
    expect(
      await restoreWorkspaceFromSession({
        version: 1,
        rootPath: null,
        tabs: [],
        activeFilePath: null,
        isSidebarCollapsed: false,
      }),
    ).toBeNull();
  });

  it("restores tree and tabs when all files exist", async () => {
    const mockFs = {
      readDirRecursive: async (path: string) => {
        expect(path).toBe("/project");
        return fakeTree;
      },
      readTextFile: async (path: string) => {
        if (path === "/project/doc1.md") return "# Doc 1";
        if (path === "/project/doc2.md") return "Plain text 2";
        throw new Error("File not found");
      },
    };

    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/project",
      tabs: [
        { filePath: "/project/doc1.md", mode: "rich" },
        { filePath: "/project/doc2.md", mode: "plain" },
      ],
      activeFilePath: "/project/doc1.md",
      isSidebarCollapsed: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored).not.toBeNull();
    expect(restored?.rootPath).toBe("/project");
    expect(restored?.tree).toEqual(fakeTree);
    expect(restored?.tabs.length).toBe(2);
    expect(restored?.tabs[0]).toEqual({
      id: "/project/doc1.md",
      filePath: "/project/doc1.md",
      title: "doc1.md",
      content: "# Doc 1",
      isDirty: false,
      mode: "rich",
    });
    expect(restored?.tabs[1]).toEqual({
      id: "/project/doc2.md",
      filePath: "/project/doc2.md",
      title: "doc2.md",
      content: "Plain text 2",
      isDirty: false,
      mode: "plain",
    });
    expect(restored?.activeTabId).toBe("/project/doc1.md");
  });

  it("gracefully skips missing files and falls back activeTabId", async () => {
    const mockFs = {
      readDirRecursive: async () => fakeTree,
      readTextFile: async (path: string) => {
        if (path === "/project/doc1.md") return "# Doc 1";
        throw new Error("File deleted");
      },
    };

    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/project",
      tabs: [
        { filePath: "/project/doc1.md", mode: "rich" },
        { filePath: "/project/deleted.md", mode: "plain" },
      ],
      activeFilePath: "/project/deleted.md", // active tab was deleted
      isSidebarCollapsed: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored).not.toBeNull();
    expect(restored?.tabs.length).toBe(1);
    expect(restored?.tabs[0].id).toBe("/project/doc1.md");
    // Falls back to the last valid tab since activeFilePath was skipped
    expect(restored?.activeTabId).toBe("/project/doc1.md");
  });

  it("resets to empty state when rootPath folder cannot be read", async () => {
    const mockFs = {
      readDirRecursive: async () => {
        throw new Error("Folder removed / inaccessible");
      },
      readTextFile: async () => "content",
    };

    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/removed-folder",
      tabs: [{ filePath: "/removed-folder/doc1.md", mode: "rich" }],
      activeFilePath: "/removed-folder/doc1.md",
      isSidebarCollapsed: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored).toEqual({
      rootPath: null,
      tree: [],
      tabs: [],
      activeTabId: null,
    });
  });

  it("handles empty tabs list with null activeTabId", async () => {
    const mockFs = {
      readDirRecursive: async () => fakeTree,
      readTextFile: async () => "content",
    };

    const session: PersistedWorkspaceSession = {
      version: 1,
      rootPath: "/project",
      tabs: [],
      activeFilePath: null,
      isSidebarCollapsed: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored?.rootPath).toBe("/project");
    expect(restored?.tabs).toEqual([]);
    expect(restored?.activeTabId).toBeNull();
  });
});

