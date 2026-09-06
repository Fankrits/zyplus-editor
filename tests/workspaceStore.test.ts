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
    roots: [],
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
        savedContent: "# Doc 1",
        isDirty: false,
        mode: "rich",
      },
      {
        id: "/project/doc2.md",
        filePath: "/project/doc2.md",
        title: "doc2.md",
        content: "Plain Doc",
        savedContent: "Plain Doc",
        isDirty: false,
        mode: "plain",
      },
    ];

    const next = workspaceReducer(initial, {
      type: "RESTORE_WORKSPACE",
      roots: ["/project"],
      tree: [{ id: "/project/doc1.md", name: "doc1.md", isFolder: false }],
      tabs,
      activeTabId: "/project/doc2.md",
    });

    expect(next.roots).toEqual(["/project"]);
    expect(next.tree.length).toBe(1);
    expect(next.tabs.length).toBe(2);
    expect(next.activeTabId).toBe("/project/doc2.md");
  });

  it("handles RESTORE_WORKSPACE with no roots and empty state", () => {
    const populated: WorkspaceState = {
      roots: ["/previous"],
      tree: [{ id: "/previous/doc.md", name: "doc.md", isFolder: false }],
      tabs: [
        {
          id: "/previous/doc.md",
          filePath: "/previous/doc.md",
          title: "doc.md",
          content: "content",
          savedContent: "saved",
          isDirty: true,
          mode: "rich",
        },
      ],
      activeTabId: "/previous/doc.md",
    };

    const next = workspaceReducer(populated, {
      type: "RESTORE_WORKSPACE",
      roots: [],
      tree: [],
      tabs: [],
      activeTabId: null,
    });

    expect(next.roots).toEqual([]);
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
  const projectNode: TreeNode = {
    id: "/project",
    name: "project",
    isFolder: true,
    children: [
      { id: "/project/doc1.md", name: "doc1.md", isFolder: false },
      { id: "/project/doc2.md", name: "doc2.md", isFolder: false },
    ],
  };

  it("returns null when session is null or has no roots", async () => {
    expect(await restoreWorkspaceFromSession(null)).toBeNull();
    expect(
      await restoreWorkspaceFromSession({
        version: 2,
        roots: [],
        defaultFolder: null,
        tabs: [],
        activeFilePath: null,
        isSidebarCollapsed: false,
        isAutosaveEnabled: false,
      }),
    ).toBeNull();
  });

  it("restores tree and tabs when all files exist", async () => {
    const mockFs = {
      readProjectNode: async (path: string) => {
        expect(path).toBe("/project");
        return projectNode;
      },
      readTextFile: async (path: string) => {
        if (path === "/project/doc1.md") return "# Doc 1";
        if (path === "/project/doc2.md") return "Plain text 2";
        throw new Error("File not found");
      },
    };

    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: ["/project"],
      defaultFolder: null,
      tabs: [
        { filePath: "/project/doc1.md", mode: "rich" },
        { filePath: "/project/doc2.md", mode: "plain" },
      ],
      activeFilePath: "/project/doc1.md",
      isSidebarCollapsed: false,
      isAutosaveEnabled: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored).not.toBeNull();
    expect(restored?.roots).toEqual(["/project"]);
    expect(restored?.tree).toEqual([projectNode]);
    expect(restored?.tabs.length).toBe(2);
    expect(restored?.tabs[0]).toEqual({
      id: "/project/doc1.md",
      filePath: "/project/doc1.md",
      title: "doc1.md",
      content: "# Doc 1",
      savedContent: "# Doc 1",
      isDirty: false,
      mode: "rich",
    });
    expect(restored?.tabs[1]).toEqual({
      id: "/project/doc2.md",
      filePath: "/project/doc2.md",
      title: "doc2.md",
      content: "Plain text 2",
      savedContent: "Plain text 2",
      isDirty: false,
      mode: "plain",
    });
    expect(restored?.activeTabId).toBe("/project/doc1.md");
  });

  it("gracefully skips missing files and falls back activeTabId", async () => {
    const mockFs = {
      readProjectNode: async () => projectNode,
      readTextFile: async (path: string) => {
        if (path === "/project/doc1.md") return "# Doc 1";
        throw new Error("File deleted");
      },
    };

    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: ["/project"],
      defaultFolder: null,
      tabs: [
        { filePath: "/project/doc1.md", mode: "rich" },
        { filePath: "/project/deleted.md", mode: "plain" },
      ],
      activeFilePath: "/project/deleted.md", // active tab was deleted
      isSidebarCollapsed: false,
      isAutosaveEnabled: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored).not.toBeNull();
    expect(restored?.tabs.length).toBe(1);
    expect(restored?.tabs[0].id).toBe("/project/doc1.md");
    // Falls back to the last valid tab since activeFilePath was skipped
    expect(restored?.activeTabId).toBe("/project/doc1.md");
  });

  it("drops projects whose folder cannot be read", async () => {
    const mockFs = {
      readProjectNode: async () => {
        throw new Error("Folder removed / inaccessible");
      },
      readTextFile: async () => "content",
    };

    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: ["/removed-folder"],
      defaultFolder: null,
      tabs: [{ filePath: "/removed-folder/doc1.md", mode: "rich" }],
      activeFilePath: "/removed-folder/doc1.md",
      isSidebarCollapsed: false,
      isAutosaveEnabled: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored?.roots).toEqual([]);
    expect(restored?.tree).toEqual([]);
  });

  it("handles empty tabs list with null activeTabId", async () => {
    const mockFs = {
      readProjectNode: async () => projectNode,
      readTextFile: async () => "content",
    };

    const session: PersistedWorkspaceSession = {
      version: 2,
      roots: ["/project"],
      defaultFolder: null,
      tabs: [],
      activeFilePath: null,
      isSidebarCollapsed: false,
      isAutosaveEnabled: false,
    };

    const restored = await restoreWorkspaceFromSession(session, mockFs);
    expect(restored?.roots).toEqual(["/project"]);
    expect(restored?.tabs).toEqual([]);
    expect(restored?.activeTabId).toBeNull();
  });
});


describe("SAVE_TAB_SUCCESS", () => {
  const dirtyTab = {
    id: "/p/a.md",
    filePath: "/p/a.md",
    title: "a.md",
    content: "typed more",
    savedContent: "typed",
    isDirty: true,
    mode: "rich" as const,
  };
  const state: WorkspaceState = {
    roots: [],
    tree: [],
    tabs: [dirtyTab],
    activeTabId: dirtyTab.id,
  };

  it("clears the dirty flag when the saved content is still current", () => {
    const next = workspaceReducer(state, {
      type: "SAVE_TAB_SUCCESS",
      id: dirtyTab.id,
      content: "typed more",
    });
    expect(next.tabs[0].isDirty).toBe(false);
  });

  it("stays dirty when the tab changed while the write was in flight", () => {
    const next = workspaceReducer(state, {
      type: "SAVE_TAB_SUCCESS",
      id: dirtyTab.id,
      content: "typed",
    });
    expect(next.tabs[0].isDirty).toBe(true);
  });
});

describe("UPDATE_TAB_CONTENT", () => {
  const tab = {
    id: "/p/a.md",
    filePath: "/p/a.md",
    title: "a.md",
    content: "on disk",
    savedContent: "on disk",
    isDirty: false,
    mode: "rich" as const,
  };
  const state: WorkspaceState = { roots: [], tree: [], tabs: [tab], activeTabId: tab.id };

  it("marks the tab dirty when the content differs from disk", () => {
    const next = workspaceReducer(state, { type: "UPDATE_TAB_CONTENT", id: tab.id, content: "edited" });
    expect(next.tabs[0].isDirty).toBe(true);
  });

  it("goes clean again when an edit is undone back to the saved content", () => {
    const edited = workspaceReducer(state, { type: "UPDATE_TAB_CONTENT", id: tab.id, content: "edited" });
    const undone = workspaceReducer(edited, { type: "UPDATE_TAB_CONTENT", id: tab.id, content: "on disk" });
    expect(undone.tabs[0].isDirty).toBe(false);
  });
});
