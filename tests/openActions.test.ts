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
      rootPath: "/demo-workspace",
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

  it("switching workspace with OPEN_ROOT resets or reassigns tree and rootPath", () => {
    const initial: WorkspaceState = {
      rootPath: "/old-project",
      tree: [{ id: "/old-project/a.md", name: "a.md", isFolder: false }],
      tabs: [{ id: "/old-project/a.md", filePath: "/old-project/a.md", title: "a.md", content: "", isDirty: false, mode: "rich" }],
      activeTabId: "/old-project/a.md",
    };

    const next = workspaceReducer(initial, {
      type: "OPEN_ROOT",
      rootPath: "/new-project",
      tree: [{ id: "/new-project/b.md", name: "b.md", isFolder: false }],
    });

    expect(next.rootPath).toBe("/new-project");
    expect(next.tree).toHaveLength(1);
    expect(next.tree[0].id).toBe("/new-project/b.md");
  });
});
