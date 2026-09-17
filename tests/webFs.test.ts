import { describe, it, expect } from "bun:test";
import * as fs from "../src/lib/fs";

// The web build has no OS underneath, so folders, moves and deletes are all
// path arithmetic over one map. These are the cases that arithmetic gets wrong.
describe("web filesystem", () => {
  it("keeps empty folders, and moves and deletes whole subtrees", async () => {
    await fs.ensureFolder("/w/empty");
    await fs.writeTextFile("/w/docs/a.md", "a");
    await fs.writeTextFile("/w/docsy.md", "not inside /w/docs");

    const tree = await fs.readDirRecursive("/w");
    expect(tree.map((n) => n.name)).toEqual(["docs", "empty", "docsy.md"]);
    expect(tree[0].children?.map((n) => n.name)).toEqual(["a.md"]);

    await fs.renamePath("/w/docs", "/w/notes");
    expect(await fs.readTextFile("/w/notes/a.md")).toBe("a");
    expect(await fs.pathExists("/w/docs")).toBe(false);
    expect(await fs.readTextFile("/w/docsy.md")).toBe("not inside /w/docs");

    expect(await fs.duplicateFile("/w/notes/a.md")).toBe("/w/notes/a copy.md");
    expect(await fs.duplicateFile("/w/notes/a.md")).toBe("/w/notes/a copy 2.md");

    await fs.deletePath("/w/notes", true);
    expect(await fs.pathExists("/w/notes/a copy.md")).toBe(false);
    expect(await fs.pathExists("/w/docsy.md")).toBe(true);
  });
});
