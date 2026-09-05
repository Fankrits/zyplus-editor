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
