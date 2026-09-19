import { describe, expect, it, beforeAll, afterAll, beforeEach } from "bun:test";
import * as fs from "../src/lib/fs";
import * as sync from "../src/lib/sync";
import { initAuth } from "../src/lib/auth";

/**
 * The sync engine against a stand-in for the real API: one shared rev sequence,
 * conflicts decided by rev, deletes as tombstones — `server/src/notes.ts` in a
 * Map. The decision table is unit-tested in `sync.test.ts`; what this file covers
 * is the loop around it, where a device has to recognise its own uploads coming
 * back and pick the right `baseRev` to retry on.
 *
 * The stand-in is installed as `globalThis.fetch`, the way `auth.test.ts` does it,
 * and the engine is signed in with a real stored token. Mocking `lib/auth` as a
 * module instead replaced it for every other test file in the run — `bun test`
 * shares one process — and whichever file happened to load after this one lost
 * the exports it imports. It passed on macOS and failed on Linux, where the files
 * load in a different order.
 */
type Row = { content: string; rev: number; deleted: boolean };
const rows = new Map<string, Row>();
let seq = 0;
let serverDown = false;

const realFetch = globalThis.fetch;

async function fakeServer(input: RequestInfo | URL, init: RequestInit = {}): Promise<Response> {
  if (serverDown) throw new Error("Can't reach the sync server");
  const url = new URL(String(input instanceof Request ? input.url : input));
  const params = url.searchParams;
  const method = init.method ?? "GET";

  if (url.pathname === "/api/notes" && method === "GET") {
    const since = Number(params.get("since") ?? 0);
    const notes = [...rows.entries()]
      .filter(([, r]) => r.rev > since)
      .sort((a, b) => a[1].rev - b[1].rev)
      .map(([relPath, r]) => ({ relPath, rev: r.rev, deletedAt: r.deleted ? "2026-01-01" : null }));
    return Response.json({ notes });
  }
  if (url.pathname === "/api/notes/content" && method === "GET") {
    const row = rows.get(params.get("path")!);
    return row
      ? Response.json({ content: row.content })
      : Response.json({ error: "Not found" }, { status: 404 });
  }
  if (url.pathname === "/api/notes" && method === "PUT") {
    const { relPath, content, baseRev } = JSON.parse(String(init.body));
    const row = rows.get(relPath);
    const wins = baseRev === 0 ? !row : row?.rev === baseRev;
    if (!wins) {
      return Response.json({ rev: row?.rev ?? 0, content: row?.content ?? "" }, { status: 409 });
    }
    rows.set(relPath, { content, rev: ++seq, deleted: false });
    return Response.json({ rev: seq });
  }
  if (url.pathname === "/api/notes" && method === "DELETE") {
    const relPath = params.get("path")!;
    const row = rows.get(relPath);
    if (row && !row.deleted) rows.set(relPath, { content: "", rev: ++seq, deleted: true });
    // Mirrors `softDelete`: a repeat hands back the tombstone already there.
    return Response.json({ rev: rows.get(relPath)?.rev ?? 0 });
  }
  throw new Error(`unhandled ${method} ${url.pathname}`);
}

beforeAll(async () => {
  globalThis.fetch = fakeServer as unknown as typeof fetch;
  // What `lib/auth` reads back outside Tauri, so the engine counts as signed in.
  localStorage.setItem("zyplus:auth-token", "test-token");
  await initAuth();
});

afterAll(async () => {
  globalThis.fetch = realFetch;
  localStorage.removeItem("zyplus:auth-token");
  await initAuth();
  sync.stopSync();
});

const USER = "user-1";

/** Points the (single, module-level) engine at one device's folder and syncs it. */
async function on(root: string): Promise<void> {
  sync.stopSync();
  await sync.startSync(root, USER);
  await sync.syncNow();
}

/** Changes the app itself did not make: the engine is not watching this folder. */
async function offline(root: string, change: () => Promise<void>): Promise<void> {
  sync.stopSync();
  await change();
  await on(root);
}

/** The device's notes, conflict copies included; the manifest folder is hidden. */
async function notesIn(root: string): Promise<string[]> {
  return (await fs.readDirRecursive(root)).map((n) => n.name).sort();
}

describe("sync engine", () => {
  beforeEach(async () => {
    sync.stopSync();
    rows.clear();
    seq = 0;
    serverDown = false;
    for (const root of ["/A", "/B"]) {
      if (await fs.pathExists(root)) await fs.deletePath(root, true);
      await fs.ensureFolder(root);
    }
  });

  it("carries a note to a second device, and edits after it", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "from A");
    await sync.syncNow();

    await on("/B");
    expect(await fs.readTextFile("/B/notes.md")).toBe("from A");

    await fs.writeTextFile("/B/notes.md", "edited on B");
    await sync.syncNow();

    await on("/A");
    expect(await fs.readTextFile("/A/notes.md")).toBe("edited on B");
    expect(await notesIn("/A")).toEqual(["notes.md"]);
  });

  it("keeps saving on one device without filing conflicts against itself", async () => {
    await on("/A");
    for (const text of ["v1", "v2", "v3"]) {
      await fs.writeTextFile("/A/notes.md", text);
      await sync.syncNow();
    }
    expect(await notesIn("/A")).toEqual(["notes.md"]);
    expect(rows.get("notes.md")!.content).toBe("v3");
  });

  it("re-creates a deleted note without an empty conflict copy", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "v1");
    await sync.syncNow();

    await fs.deletePath("/A/notes.md", false);
    await sync.syncNow();

    await fs.writeTextFile("/A/notes.md", "fresh");
    await sync.syncNow();

    expect(await notesIn("/A")).toEqual(["notes.md"]);
    expect(rows.get("notes.md")).toMatchObject({ content: "fresh", deleted: false });
  });

  it("propagates a delete to the other device", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "v1");
    await sync.syncNow();
    await on("/B");
    expect(await notesIn("/B")).toEqual(["notes.md"]);

    await on("/A");
    await fs.deletePath("/A/notes.md", false);
    await sync.syncNow();

    await on("/B");
    expect(await notesIn("/B")).toEqual([]);
  });

  it("lets the cloud win a real conflict, keeping the local edit in one copy", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "shared");
    await sync.syncNow();
    await on("/B");

    // Both devices edit the same note before either has seen the other's change:
    // A's edit lands on disk while the engine is still pointed at B.
    await fs.writeTextFile("/B/notes.md", "B's edit");
    await sync.syncNow();
    await fs.writeTextFile("/A/notes.md", "A's edit");
    await on("/A");

    const names = await notesIn("/A");
    expect(names).toHaveLength(2);
    expect(await fs.readTextFile("/A/notes.md")).toBe("B's edit");
    const copy = names.find((n) => n !== "notes.md")!;
    expect(await fs.readTextFile(`/A/${copy}`)).toBe("A's edit");
    // The cloud keeps the live note, and the local edit goes up as its own note.
    expect(rows.get("notes.md")!.content).toBe("B's edit");
    expect(rows.get(copy)!.content).toBe("A's edit");
    await sync.syncNow();
    expect(await notesIn("/A")).toEqual(names);
  });

  it("files no conflict for a note whose content already matches the cloud", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "same everywhere");
    await sync.syncNow();

    // B already has the file — copied over by hand — but no manifest for it.
    await fs.writeTextFile("/B/notes.md", "same everywhere");
    await on("/B");

    expect(await notesIn("/B")).toEqual(["notes.md"]);
    expect(rows.size).toBe(1);
  });

  it("uploads a note edited outside the app", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "v1");
    await sync.syncNow();

    // Another editor writes to the folder while Zyplus is closed.
    await offline("/A", () => fs.writeTextFile("/A/notes.md", "edited in vim"));

    expect(rows.get("notes.md")!.content).toBe("edited in vim");
    expect(await notesIn("/A")).toEqual(["notes.md"]);
  });

  it("uploads a note added to the folder outside the app", async () => {
    await on("/A");
    await offline("/A", () => fs.writeTextFile("/A/dropped-in.md", "copied here"));

    await on("/B");
    expect(await fs.readTextFile("/B/dropped-in.md")).toBe("copied here");
  });

  it("propagates a note deleted outside the app", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "v1");
    await sync.syncNow();
    await on("/B");
    expect(await notesIn("/B")).toEqual(["notes.md"]);

    await offline("/A", () => fs.deletePath("/A/notes.md", false));
    expect(rows.get("notes.md")!.deleted).toBe(true);

    await on("/B");
    expect(await notesIn("/B")).toEqual([]);
  });

  it("re-creates a note the server already tombstoned twice", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "v1");
    await sync.syncNow();
    await fs.deletePath("/A/notes.md", false);
    await sync.syncNow();
    // A second delete for the same path — a retry, or another device getting there
    // first — must still leave a rev the next upload can build on.
    await sync.startSync("/A", USER);
    await fs.writeTextFile("/A/notes.md", "back again");
    await sync.syncNow();

    expect(await notesIn("/A")).toEqual(["notes.md"]);
    expect(rows.get("notes.md")).toMatchObject({ content: "back again", deleted: false });
  });

  it("uploads work written while the server was unreachable", async () => {
    await on("/A");

    serverDown = true;
    await fs.writeTextFile("/A/notes.md", "written offline");
    await sync.syncNow();
    expect(rows.has("notes.md")).toBe(false);

    serverDown = false;
    await sync.syncNow();
    expect(rows.get("notes.md")!.content).toBe("written offline");
  });

  it("brings a note back when one device deletes it and the other edited it", async () => {
    await on("/A");
    await fs.writeTextFile("/A/notes.md", "shared");
    await sync.syncNow();
    await on("/B");

    await fs.deletePath("/B/notes.md", false);
    await sync.syncNow();

    // A edited it before hearing about the delete, so the edit has to survive.
    await fs.writeTextFile("/A/notes.md", "still wanted");
    await on("/A");

    expect(await notesIn("/A")).toEqual(["notes.md"]);
    expect(rows.get("notes.md")).toMatchObject({ content: "still wanted", deleted: false });
  });

  it("a sync stopped mid-run bails quietly instead of reporting an error", async () => {
    rows.set("n.md", { content: "remote", rev: ++seq, deleted: false });
    await sync.startSync("/A", USER);
    const running = sync.syncNow();
    sync.stopSync();
    await running;
    expect(sync.getStatus().state).toBe("off");
    expect(await fs.pathExists("/A/n.md")).toBe(false);
  });
});
