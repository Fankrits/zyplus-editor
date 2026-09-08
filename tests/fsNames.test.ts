import { describe, expect, it } from "bun:test";
import { explainFsError, invalidNameReason } from "../src/lib/fs";

describe("invalidNameReason", () => {
  it("accepts ordinary names", () => {
    for (const name of ["notes.md", "my-note.md", "réunion.md", "a.b.md", "CONTACTS.md"]) {
      expect(invalidNameReason(name)).toBeNull();
    }
  });

  it("rejects characters Windows cannot store", () => {
    for (const name of ['a:b.md', "a/b", "a\\b", "a<b", "a>b", 'a"b', "a|b", "a?b", "a*b"]) {
      expect(invalidNameReason(name)).not.toBeNull();
    }
  });

  it("rejects names Windows reserves for devices", () => {
    for (const name of ["CON", "con.md", "NUL", "com1", "LPT9.txt"]) {
      expect(invalidNameReason(name)).not.toBeNull();
    }
  });

  it("rejects trailing periods and spaces, which Windows silently drops", () => {
    expect(invalidNameReason("notes.")).not.toBeNull();
    expect(invalidNameReason("notes ")).not.toBeNull();
  });

  it("rejects empty, dot and control-character names", () => {
    expect(invalidNameReason("")).not.toBeNull();
    expect(invalidNameReason(".")).not.toBeNull();
    expect(invalidNameReason("..")).not.toBeNull();
    expect(invalidNameReason("a\u0007b")).not.toBeNull();
  });
});

describe("explainFsError", () => {
  it("turns a permission denial into guidance naming the path", () => {
    const out = explainFsError(new Error("Access is denied. (os error 5)"), "C:\\x\\Zyplus");
    expect(out).toContain("C:\\x\\Zyplus");
    // The raw errno is useless to a user; the explanation must replace it.
    expect(out).not.toContain("os error 5");
    expect(out.length).toBeGreaterThan(60);
  });

  it("recognises the POSIX spellings too", () => {
    for (const raw of ["EACCES: permission denied", "EPERM: operation not permitted"]) {
      expect(explainFsError(new Error(raw), "/x")).not.toBe(raw);
    }
  });

  it("passes an unrecognised error through untouched", () => {
    expect(explainFsError(new Error('"/x/a.md" already exists'), "/x/a.md")).toBe(
      '"/x/a.md" already exists',
    );
  });
});

describe("renamePath name guard", () => {
  it("rejects a retyped name that Windows cannot store", async () => {
    const { renamePath } = await import("../src/lib/fs");
    expect(renamePath("/x/notes.md", "/x/a:b.md")).rejects.toThrow();
  });

  it("still moves a file that already has an awkward name", async () => {
    const { renamePath } = await import("../src/lib/fs");
    // Same basename, different folder: the name is pre-existing, not chosen now.
    await renamePath("/x/a:b.md", "/y/a:b.md");
  });
});

describe("tryFs", () => {
  it("reports a failure as false rather than throwing", async () => {
    const { tryFs } = await import("../src/lib/fs");
    // Every call site keys off this: a true here over a failed write would
    // refresh the tree and open a tab for a file that does not exist.
    expect(await tryFs("Nope", "/x", () => Promise.reject(new Error("boom")))).toBe(false);
    expect(await tryFs("Fine", "/x", () => Promise.resolve())).toBe(true);
  });
});
