import { describe, it, expect } from "bun:test";
import { openFileDialog, openFolderDialog, readTextFile } from "../src/lib/fs";
import { workspaceReducer, type WorkspaceState, type TabState } from "../src/state/workspaceStore";

describe("open actions & workspace transitions", () => {
  it("openFileDialog returns mock path when not in Tauri environment", async () => {
    const file = await openFileDialog();
    expect(file).toBe("/demo-workspace/README.md");
    if (file) {
      const content = await readTextFile(file);
      expect(content).toContain("Tauri + React + Typescript");
    }
  });

  it("openFolderDialog returns mock root path when not in Tauri environment", async () => {
    const folder = await openFolderDialog();
    expect(folder).toBe("/demo-workspace");
  });

  it("opening an external file adds a new tab and focuses it", () => {
    const initial: WorkspaceState = {
      roots: ["/demo-workspace"],
      tree: [],
      tabs: [],
      activeTabId: null,
    };

    const newTab: TabState = {
      id: "/external/notes.md",
      filePath: "/external/notes.md",
      title: "notes.md",
      content: "# External notes",
      isDirty: false,
      mode: "rich",
    };

    const next = workspaceReducer(initial, {
      type: "OPEN_TAB",
      tab: newTab,
    });

    expect(next.tabs).toHaveLength(1);
    expect(next.tabs[0].id).toBe("/external/notes.md");
    expect(next.activeTabId).toBe("/external/notes.md");
  });

  it("ADD_ROOT appends a project without touching the existing one, and ignores duplicates", () => {
    const initial: WorkspaceState = {
      roots: ["/old-project"],
      tree: [{ id: "/old-project", name: "old-project", isFolder: true, children: [] }],
      tabs: [{ id: "/old-project/a.md", filePath: "/old-project/a.md", title: "a.md", content: "", isDirty: false, mode: "rich" }],
      activeTabId: "/old-project/a.md",
    };

    const next = workspaceReducer(initial, {
      type: "ADD_ROOT",
      rootPath: "/new-project",
      node: { id: "/new-project", name: "new-project", isFolder: true, children: [] },
    });

    expect(next.roots).toEqual(["/old-project", "/new-project"]);
    expect(next.tree).toHaveLength(2);
    expect(next.tabs).toHaveLength(1);

    const again = workspaceReducer(next, {
      type: "ADD_ROOT",
      rootPath: "/new-project",
      node: { id: "/new-project", name: "new-project", isFolder: true, children: [] },
    });
    expect(again).toBe(next);
  });

  it("CLOSE_ROOT drops the project, its tree node and its open tabs", () => {
    const initial: WorkspaceState = {
      roots: ["/a", "/b"],
      tree: [
        { id: "/a", name: "a", isFolder: true, children: [] },
        { id: "/b", name: "b", isFolder: true, children: [] },
      ],
      tabs: [
        { id: "/a/one.md", filePath: "/a/one.md", title: "one.md", content: "", isDirty: false, mode: "rich" },
        { id: "/b/two.md", filePath: "/b/two.md", title: "two.md", content: "", isDirty: false, mode: "rich" },
      ],
      activeTabId: "/a/one.md",
    };

    const next = workspaceReducer(initial, { type: "CLOSE_ROOT", rootPath: "/a" });
    expect(next.roots).toEqual(["/b"]);
    expect(next.tree.map((n) => n.id)).toEqual(["/b"]);
    expect(next.tabs.map((t) => t.id)).toEqual(["/b/two.md"]);
    expect(next.activeTabId).toBe("/b/two.md");
  });
});
