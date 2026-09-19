import { describe, expect, it } from "bun:test";
import {
  conflictName,
  decidePull,
  describeFailedPush,
  hashOf,
  toAbsPath,
  toRelPath,
} from "../src/lib/sync";

/**
 * The decision table is the whole of sync's correctness: every row that resolves
 * to "download" or "delete" discards local content, so a wrong answer here is
 * silent data loss rather than a visible failure.
 */
describe("decidePull", () => {
  const base = {
    remoteDeleted: false,
    localExists: true,
    localHash: "a",
    knownHash: "a",
    remoteHash: "b" as string | null,
  };

  it("applies a remote edit when the local file is untouched", () => {
    expect(decidePull(base)).toBe("download");
  });

  it("downloads a file this device has never seen", () => {
    expect(decidePull({ ...base, localExists: false, localHash: null })).toBe("download");
  });

  it("moves the local edit aside when both sides changed", () => {
    expect(decidePull({ ...base, localHash: "local-edit" })).toBe("conflict");
  });

  it("treats a never-synced local file as an edit, not as stale", () => {
    // No manifest entry — after a reinstall, say. Downloading over it would
    // throw away work that was never uploaded.
    expect(decidePull({ ...base, knownHash: null })).toBe("conflict");
  });

  it("never files a conflict when disk already holds the cloud content", () => {
    // A lost manifest, or the folder copied to a new machine: nothing diverged.
    expect(decidePull({ ...base, knownHash: null, localHash: "b" })).toBe("adopt");
    expect(decidePull({ ...base, localHash: "b", knownHash: "old" })).toBe("adopt");
  });

  it("propagates a delete when the local file is untouched", () => {
    expect(decidePull({ ...base, remoteDeleted: true, remoteHash: null })).toBe("delete");
  });

  it("refuses a delete when the local file was edited since", () => {
    expect(
      decidePull({ ...base, remoteDeleted: true, remoteHash: null, localHash: "local-edit" }),
    ).toBe("keep-local");
  });

  it("does nothing when a delete arrives for a file already gone", () => {
    expect(
      decidePull({ ...base, remoteDeleted: true, remoteHash: null, localExists: false, localHash: null }),
    ).toBe("none");
  });
});

describe("conflictName", () => {
  const day = new Date("2026-09-08T12:00:00Z");

  it("names the copy beside the original", () => {
    expect(conflictName("notes.md", day)).toBe("notes (conflict 2026-09-08).md");
  });

  it("keeps the copy in the same folder", () => {
    expect(conflictName("projects/spec.md", day)).toBe("projects/spec (conflict 2026-09-08).md");
  });

  it("numbers a second conflict on the same day", () => {
    expect(conflictName("notes.md", day, 1)).toBe("notes (conflict 2026-09-08 2).md");
  });

  it("handles a file with no extension", () => {
    expect(conflictName("LICENSE", day)).toBe("LICENSE (conflict 2026-09-08)");
  });
});

describe("path mapping", () => {
  it("round-trips a nested path", () => {
    const root = "/Users/me/Documents/Zyplus";
    const abs = toAbsPath(root, "projects/spec.md");
    expect(abs).toBe("/Users/me/Documents/Zyplus/projects/spec.md");
    expect(toRelPath(root, abs)).toBe("projects/spec.md");
  });

  it("stores forward slashes but rebuilds Windows separators", () => {
    const root = "C:\\Users\\me\\Zyplus";
    expect(toRelPath(root, "C:\\Users\\me\\Zyplus\\projects\\spec.md")).toBe("projects/spec.md");
    expect(toAbsPath(root, "projects/spec.md")).toBe("C:\\Users\\me\\Zyplus\\projects\\spec.md");
  });

  it("rejects paths outside the synced folder", () => {
    // Otherwise opening an unrelated project would upload it.
    expect(toRelPath("/home/me/Zyplus", "/home/me/other/notes.md")).toBeNull();
    expect(toRelPath("/home/me/Zyplus", "/home/me/Zyplus-backup/notes.md")).toBeNull();
    expect(toRelPath("/home/me/Zyplus", "/home/me/Zyplus")).toBeNull();
  });
});

describe("hashOf", () => {
  it("separates identical from changed content", async () => {
    expect(await hashOf("# hello")).toBe(await hashOf("# hello"));
    expect(await hashOf("# hello")).not.toBe(await hashOf("# hello "));
  });
});

describe("describeFailedPush", () => {
  it("keeps the retry wording for ordinary failures", () => {
    expect(describeFailedPush(3, false)).toBe("Some changes are waiting to upload");
  });

  it("tells the user to free space when storage is full, since retrying can't fix it", () => {
    expect(describeFailedPush(1, true)).toContain("1 change can't upload");
    expect(describeFailedPush(4, true)).toContain("4 changes can't upload");
    expect(describeFailedPush(4, true)).toContain("Delete some synced notes");
  });
});
