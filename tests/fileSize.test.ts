import { describe, it, expect } from "bun:test";
import {
  MAX_OPEN_SIZE,
  RICH_TEXT_LIMIT,
  formatSize,
  isTooLargeToOpen,
  modeForSize,
  tooLargeMessage,
  utf8Length,
} from "../src/lib/fileSize";
import { fileSize, writeTextFile } from "../src/lib/fs";
import { restoreWorkspaceFromSession } from "../src/state/workspaceStore";
import type { PersistedWorkspaceSession } from "../src/lib/sessionStorage";

describe("open size policy", () => {
  it("keeps small files in the editor they asked for", () => {
    expect(modeForSize(1024, "rich")).toBe("rich");
    expect(modeForSize(RICH_TEXT_LIMIT, "rich")).toBe("rich");
    expect(modeForSize(1024, "plain")).toBe("plain");
  });

  it("opens big files in the source editor", () => {
    expect(modeForSize(RICH_TEXT_LIMIT + 1, "rich")).toBe("plain");
    expect(modeForSize(MAX_OPEN_SIZE, "plain")).toBe("plain");
  });

  it("refuses only what is over the hard limit", () => {
    expect(isTooLargeToOpen(MAX_OPEN_SIZE)).toBe(false);
    expect(isTooLargeToOpen(MAX_OPEN_SIZE + 1)).toBe(true);
  });

  it("formats sizes and explains a refusal", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(2048)).toBe("2 KB");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(tooLargeMessage(30 * 1024 * 1024)).toContain("30.0 MB");
  });
});

describe("utf8Length", () => {
  it("matches the encoder for ASCII, multi-byte and surrogate-pair text", () => {
    for (const text of ["", "plain ascii", "héllo wörld", "日本語のノート", "emoji 😀 and 🎉", "lone \ud800 surrogate"]) {
      expect(utf8Length(text)).toBe(new TextEncoder().encode(text).length);
    }
  });
});

describe("fileSize (web filesystem)", () => {
  it("counts UTF-8 bytes, not characters", async () => {
    await writeTextFile("/size-test.md", "héllo");
    expect(await fileSize("/size-test.md")).toBe(6);
  });

  it("rejects a file that does not exist", async () => {
    await expect(fileSize("/no-such-file.md")).rejects.toThrow();
  });
});

describe("restoring a session with big files", () => {
  const session: PersistedWorkspaceSession = {
    version: 2,
    roots: [],
    defaultFolder: null,
    tabs: [
      { filePath: "/small.md", mode: "rich" },
      { filePath: "/big.md", mode: "rich" },
      { filePath: "/huge.md", mode: "rich" },
    ],
    activeFilePath: "/small.md",
    isSidebarCollapsed: false,
    isAutosaveEnabled: false,
  };
  const files: Record<string, string> = {
    "/small.md": "# hi",
    "/big.md": "x".repeat(RICH_TEXT_LIMIT + 1),
    "/huge.md": "x".repeat(MAX_OPEN_SIZE + 1),
  };

  it("keeps the mode a big file was left in, and drops a huge one", async () => {
    const restored = await restoreWorkspaceFromSession(session, {
      readProjectNode: async (dir) => ({ id: dir, name: dir, isFolder: true, children: [] }),
      readTextFile: async (path) => files[path],
    });
    expect(restored?.tabs.map((t) => [t.title, t.mode])).toEqual([
      ["small.md", "rich"],
      ["big.md", "rich"],
    ]);
  });

  it("checks the size before reading, so a huge file is never read", async () => {
    const read: string[] = [];
    const restored = await restoreWorkspaceFromSession(session, {
      readProjectNode: async (dir) => ({ id: dir, name: dir, isFolder: true, children: [] }),
      fileSize: async (path) => files[path].length,
      readTextFile: async (path) => {
        read.push(path);
        return files[path];
      },
    });
    expect(read).not.toContain("/huge.md");
    expect(restored?.tabs.map((t) => t.title)).toEqual(["small.md", "big.md"]);
  });

  it("drops a tab whose size cannot be read, like one whose file is gone", async () => {
    const restored = await restoreWorkspaceFromSession(session, {
      readProjectNode: async (dir) => ({ id: dir, name: dir, isFolder: true, children: [] }),
      fileSize: async (path) => {
        if (path === "/big.md") throw new Error("gone");
        return files[path].length;
      },
      readTextFile: async (path) => files[path],
    });
    expect(restored?.tabs.map((t) => t.title)).toEqual(["small.md"]);
  });
});
