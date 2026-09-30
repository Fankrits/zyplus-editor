import { describe, it, expect } from "bun:test";
import * as fs from "../src/lib/fs";
import { SESSION_STORAGE_KEY, clearSession, loadSession, saveSession } from "../src/lib/sessionStorage";
import {
  restoreWorkspaceFromSession,
  findTreeNode,
  loadedFolderIds,
  workspaceReducer,
  type TreeNode,
  type WorkspaceState,
} from "../src/state/workspaceReducer";

// A project is read one level at a time: what the user has opened is on screen, the rest
// stays unread. These pin the parts a refresh and a folder's first open depend on.
describe("lazy file tree", () => {
  it("reads a project's first level only, and reads back the folders that were opened", async () => {
    await fs.writeTextFile("/lz/a/b/deep.md", "x");
    await fs.writeTextFile("/lz/a/top.md", "x");
    await fs.writeTextFile("/lz/c/other.md", "x");

    const first = await fs.readProjectNode("/lz");
    expect(first.children?.map((n) => [n.name, n.unloaded ?? false])).toEqual([["a", true], ["c", true]]);

    // A refresh with `a` opened reads `a` again, and nothing below it or beside it.
    const again = await fs.readProjectNode("/lz", new Set(["/lz", "/lz/a"]));
    const a = again.children?.[0];
    expect(a?.unloaded).toBeUndefined();
    expect(a?.children?.map((n) => [n.name, n.unloaded ?? false])).toEqual([["b", true], ["top.md", false]]);
    expect(again.children?.[1].unloaded).toBe(true);
  });

  it("leaves a folder unread when it vanished between listing and reading", async () => {
    await fs.writeTextFile("/gone/x/y.md", "x");
    const listed = await fs.readChildren("/gone");
    await fs.deletePath("/gone/x", true);
    // `x` was listed, then removed; reading it again must not fail the whole refresh.
    const node = await fs.readProjectNode("/gone", new Set(["/gone/x"]));
    expect(listed[0].name).toBe("x");
    expect(node.children).toEqual([]);
  });

  const folder = (id: string, children: TreeNode[] = [], unloaded = false): TreeNode => ({
    id,
    name: id.split("/").pop()!,
    isFolder: true,
    children,
    ...(unloaded ? { unloaded } : {}),
  });
  const file = (id: string): TreeNode => ({ id, name: id.split("/").pop()!, isFolder: false });

  const state = (tree: TreeNode[]): WorkspaceState => ({ roots: ["/p"], tree, tabs: [], activeTabId: null });

  it("SET_CHILDREN fills one folder and shares every subtree it did not touch", () => {
    const untouched = folder("/p/other", [file("/p/other/o.md")]);
    const before = state([folder("/p", [folder("/p/a", [], true), untouched])]);

    const after = workspaceReducer(before, { type: "SET_CHILDREN", id: "/p/a", children: [file("/p/a/n.md")] });

    const a = findTreeNode(after.tree, "/p/a");
    expect(a?.children?.map((n) => n.name)).toEqual(["n.md"]);
    expect(a?.unloaded).toBeUndefined();
    expect(findTreeNode(after.tree, "/p/other")).toBe(untouched);
  });

  it("SET_CHILDREN for a folder that is no longer in the tree changes nothing", () => {
    const before = state([folder("/p", [folder("/p/a", [], true)])]);
    expect(workspaceReducer(before, { type: "SET_CHILDREN", id: "/p/zzz", children: [] })).toBe(before);
  });

  it("does not mistake a sibling with a shared prefix for an ancestor", () => {
    const before = state([folder("/p", [folder("/p/ab", [file("/p/ab/1.md")]), folder("/p/a", [], true)])]);
    const after = workspaceReducer(before, { type: "SET_CHILDREN", id: "/p/a", children: [file("/p/a/2.md")] });
    expect(findTreeNode(after.tree, "/p/ab")?.children?.map((n) => n.name)).toEqual(["1.md"]);
    expect(findTreeNode(after.tree, "/p/a")?.children?.map((n) => n.name)).toEqual(["2.md"]);
  });

  it("loadedFolderIds lists roots and opened folders, not unread ones or files", () => {
    const tree = [folder("/p", [folder("/p/open", [folder("/p/open/deeper", [], true), file("/p/open/f.md")]), folder("/p/closed", [], true), file("/p/r.md")])];
    expect([...loadedFolderIds(tree)].sort()).toEqual(["/p", "/p/open"]);
  });
});

describe("open folders survive a restart", () => {
  const stored = (extra: object) => ({
    version: 2 as const,
    roots: ["/p"],
    defaultFolder: null,
    tabs: [],
    activeFilePath: null,
    isSidebarCollapsed: false,
    isAutosaveEnabled: false,
    ...extra,
  });

  it("asks for the folders that were open, and only those", async () => {
    const seen: ReadonlySet<string>[] = [];
    const api = {
      readProjectNode: async (dir: string, loaded: ReadonlySet<string> = new Set()) => {
        seen.push(loaded);
        return { id: dir, name: "p", isFolder: true, children: [] };
      },
      readTextFile: async () => "",
    };
    await restoreWorkspaceFromSession(stored({ openFolders: ["/p", "/p/a"] }), api);
    expect([...seen[0]].sort()).toEqual(["/p", "/p/a"]);
  });

  it("treats a session saved before this was kept as having its projects open", async () => {
    const seen: ReadonlySet<string>[] = [];
    const api = {
      readProjectNode: async (dir: string, loaded: ReadonlySet<string> = new Set()) => {
        seen.push(loaded);
        return { id: dir, name: "p", isFolder: true, children: [] };
      },
      readTextFile: async () => "",
    };
    await restoreWorkspaceFromSession(stored({}), api);
    expect([...seen[0]]).toEqual(["/p"]);
  });

  it("keeps openFolders through save and load, and ignores a malformed one", () => {
    saveSession(stored({ openFolders: ["/p", "/p/a"] }));
    expect(loadSession()?.openFolders).toEqual(["/p", "/p/a"]);
    localStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify(stored({ openFolders: [1, 2] })));
    expect(loadSession()?.openFolders).toBeUndefined();
    clearSession();
  });
});
