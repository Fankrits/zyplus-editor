import { useSyncExternalStore } from "react";
import { authFetch, isSignedIn } from "./auth";
import {
  deletePath,
  ensureFolder,
  isMarkdownFile,
  pathExists,
  readDirRecursive,
  readTextFile,
  setLocalChangeListener,
  writeTextFile,
} from "./fs";
import type { TreeNode } from "../state/workspaceReducer";

export const MANIFEST_DIR = ".zyplus";
export const MANIFEST_FILE = "sync.json";

const PUSH_DEBOUNCE_MS = 2000;
const PULL_INTERVAL_MS = 60 * 1000;

/**
 * What this device believes the server holds. Lives inside the synced folder so
 * that reinstalling the app — or moving the folder to a new machine alongside
 * its files — does not make every file look locally edited and produce a
 * conflict copy of the entire notebook.
 */
export interface SyncManifest {
  userId: string;
  /** Highest `rev` pulled. The server returns only rows above it. */
  cursor: number;
  files: Record<string, { hash: string; rev: number }>;
}

/**
 * Recorded as a file's hash when this device holds that revision but its disk
 * copy is deliberately something else — a tombstone, or local content that beat
 * the server. It is never a real hash, so `decidePull` keeps treating the file
 * as locally edited, while the rev still says which revision to upload on top of.
 */
const UNSYNCED = "";

export interface SyncStatus {
  /** `waiting`: another tab holds the sync lock, so this one is standing by. */
  state: "off" | "idle" | "syncing" | "waiting" | "error";
  lastSyncedAt: number | null;
  error: string | null;
}

/* ------------------------------------------------------------------ *
 * Pure core                                                           *
 * ------------------------------------------------------------------ */

export type PullAction = "download" | "conflict" | "delete" | "keep-local" | "none";

/**
 * What to do with one changed remote note.
 *
 * The whole thing turns on a single question: is the local file still exactly
 * what the server last gave us? If it is, the remote copy is strictly newer and
 * can be applied. If it is not, both sides moved and neither may be discarded —
 * local content stays put and the remote copy lands beside it.
 */
export function decidePull(args: {
  remoteDeleted: boolean;
  localExists: boolean;
  /** Hash of the file on disk now; null when it is not there. */
  localHash: string | null;
  /** Hash recorded at the last successful sync; null when never synced. */
  knownHash: string | null;
}): PullAction {
  const { remoteDeleted, localExists, localHash, knownHash } = args;
  const localUntouched = localHash !== null && localHash === knownHash;

  if (remoteDeleted) {
    if (!localExists) return "none";
    return localUntouched ? "delete" : "keep-local";
  }
  if (!localExists) return "download";
  return localUntouched ? "download" : "conflict";
}

/**
 * The sync status after some uploads failed. Running out of storage gets its own
 * message because, unlike a dropped connection, retrying cannot fix it: the user
 * has to free up space first. Failed uploads stay queued either way, so they go
 * through on the first sync after space is freed.
 */
export function describeFailedPush(failed: number, storageFull: boolean): string {
  if (!storageFull) return "Some changes are waiting to upload";
  const changes = failed === 1 ? "1 change" : `${failed} changes`;
  return `Storage is full, so ${changes} can't upload. Delete some synced notes to make room.`;
}

/** `notes.md` → `notes (conflict 2026-09-08).md`. */
export function conflictName(relPath: string, date: Date, suffix = 0): string {
  const slash = relPath.lastIndexOf("/");
  const dir = slash === -1 ? "" : relPath.slice(0, slash + 1);
  const name = relPath.slice(slash + 1);
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  const day = date.toISOString().slice(0, 10);
  const n = suffix > 0 ? ` ${suffix + 1}` : "";
  return `${dir}${stem} (conflict ${day}${n})${ext}`;
}

export async function hashOf(content: string): Promise<string> {
  const buf = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(content));
  return [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/* ------------------------------------------------------------------ *
 * Paths                                                               *
 * ------------------------------------------------------------------ */

/** Windows hands back `\`; the server only ever stores `/`. */
function separatorOf(root: string): string {
  return root.includes("\\") ? "\\" : "/";
}

export function toRelPath(root: string, absPath: string): string | null {
  const normalise = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const nRoot = normalise(root);
  const nAbs = normalise(absPath);
  if (nAbs === nRoot || !nAbs.startsWith(`${nRoot}/`)) return null;
  return nAbs.slice(nRoot.length + 1);
}

export function toAbsPath(root: string, relPath: string): string {
  const sep = separatorOf(root);
  return `${root.replace(/[\\/]+$/, "")}${sep}${relPath.split("/").join(sep)}`;
}

function parentOf(absPath: string): string {
  const cut = Math.max(absPath.lastIndexOf("/"), absPath.lastIndexOf("\\"));
  return cut <= 0 ? absPath : absPath.slice(0, cut);
}

/* ------------------------------------------------------------------ *
 * Engine                                                              *
 * ------------------------------------------------------------------ */

let root: string | null = null;
let manifest: SyncManifest | null = null;
let pending = new Set<string>();
let pushTimer: ReturnType<typeof setTimeout> | null = null;
let pullTimer: ReturnType<typeof setInterval> | null = null;
let inFlight: Promise<void> | null = null;

/**
 * Bumped by every `startSync`/`stopSync`. `startSync` awaits the manifest, and
 * in that gap the folder or account can change — under StrictMode it always
 * does — so a run that is no longer current has to drop out rather than install
 * a second timer over the live one.
 */
let generation = 0;

/** An upload the server refused because the account is out of storage. */
class StorageFullError extends Error {}

/**
 * Held for as long as this window is the one syncing, and dropped by `stopSync`
 * or by the browser when the tab closes.
 *
 * Every tab of the web build shares one notes folder and one manifest, so two
 * engines would overwrite each other's bookkeeping. The others wait in line
 * rather than give up, which is what makes closing the syncing tab hand the job
 * to a remaining one instead of leaving nobody syncing until a reload. Their
 * edits go up either way: the holder's `scanLocal` finds them on its next pull.
 */
let releaseSyncLock: (() => void) | null = null;
let lockWait: AbortController | null = null;

/**
 * Writes made by sync itself must not bounce straight back as uploads, so the
 * change listener stands down while remote content is being applied.
 */
let isApplyingRemote = false;

let status: SyncStatus = { state: "off", lastSyncedAt: null, error: null };
const statusListeners = new Set<(s: SyncStatus) => void>();
const pulledListeners = new Set<(changedPaths: string[]) => void>();

export function getStatus(): SyncStatus {
  return status;
}

export function onStatus(cb: (s: SyncStatus) => void): () => void {
  statusListeners.add(cb);
  return () => statusListeners.delete(cb);
}

export function useSyncStatus(): SyncStatus {
  return useSyncExternalStore(onStatus, getStatus, getStatus);
}

/** Fires after a pull touched disk, with the absolute paths that changed. */
export function onPulled(cb: (changedPaths: string[]) => void): () => void {
  pulledListeners.add(cb);
  return () => pulledListeners.delete(cb);
}

function setStatus(next: Partial<SyncStatus>): void {
  status = { ...status, ...next };
  for (const cb of statusListeners) cb(status);
}

function manifestPath(): string {
  return toAbsPath(root!, `${MANIFEST_DIR}/${MANIFEST_FILE}`);
}

async function loadManifest(userId: string): Promise<SyncManifest> {
  const empty: SyncManifest = { userId, cursor: 0, files: {} };
  try {
    const raw = await readTextFile(manifestPath());
    const parsed = JSON.parse(raw) as SyncManifest;
    // A different account on the same folder starts over rather than inheriting
    // revisions that mean nothing to it.
    if (parsed.userId !== userId) return empty;
    return {
      userId,
      cursor: Number(parsed.cursor) || 0,
      files: parsed.files && typeof parsed.files === "object" ? parsed.files : {},
    };
  } catch {
    return empty;
  }
}

async function saveManifest(): Promise<void> {
  if (!manifest || !root) return;
  try {
    await ensureFolder(toAbsPath(root, MANIFEST_DIR));
    await writeTextFile(manifestPath(), JSON.stringify(manifest, null, 2));
  } catch (err) {
    console.warn("Failed to write the sync manifest:", err);
  }
}

/* --- push --- */

/**
 * Schedules the upload. It goes through `syncNow` rather than straight to
 * `flush`, so a push can never run alongside a pull — the two racing on one file
 * uploaded content the pull was still writing. Waiting rather than joining the
 * run in progress is what keeps the debounce: that run read `pending` before
 * this path was added to it.
 */
function queue(relPath: string): void {
  pending.add(relPath);
  if (pushTimer) clearTimeout(pushTimer);
  pushTimer = setTimeout(function fire() {
    if (inFlight) {
      pushTimer = setTimeout(fire, PUSH_DEBOUNCE_MS);
      return;
    }
    void syncNow();
  }, PUSH_DEBOUNCE_MS);
}

function handleLocalChange(absPath: string): void {
  if (isApplyingRemote || !root || !isSignedIn()) return;
  const rel = toRelPath(root, absPath);
  // Only text documents are mirrored, and never sync's own bookkeeping.
  if (!rel || rel.startsWith(`${MANIFEST_DIR}/`) || !isMarkdownFile(rel)) return;
  queue(rel);
}

async function pushOne(relPath: string): Promise<void> {
  const abs = toAbsPath(root!, relPath);
  const known = manifest!.files[relPath];

  if (!(await pathExists(abs))) {
    const res = await authFetch(`/api/notes?path=${encodeURIComponent(relPath)}`, {
      method: "DELETE",
    });
    if (!res.ok) throw new Error(`Delete of ${relPath} failed (${res.status})`);
    const { rev } = (await res.json()) as { rev: number };
    // Remember the tombstone rather than forgetting the path. Re-creating a note
    // with the same name then uploads on top of the tombstone; forgetting it made
    // the upload start from rev 0, collide with the tombstone, and leave an empty
    // conflict copy beside the new note.
    manifest!.files[relPath] = { hash: UNSYNCED, rev };
    return;
  }

  const content = await readTextFile(abs);
  const send = (baseRev: number) =>
    authFetch("/api/notes", {
      method: "PUT",
      body: JSON.stringify({ relPath, content, baseRev }),
    });

  let res = await send(known?.rev ?? 0);

  if (res.status === 409) {
    // Both sides moved. Local content is what the user is looking at, so it
    // wins; the server's copy is preserved beside it and the upload is retried
    // on top of the revision that beat us.
    const loser = (await res.json()) as { rev: number; content: string };
    await writeConflictCopy(relPath, loser.content);
    res = await send(loser.rev);
  }

  if (res.status === 413) throw new StorageFullError(`No storage left for ${relPath}`);
  if (!res.ok) throw new Error(`Upload of ${relPath} failed (${res.status})`);
  const { rev } = (await res.json()) as { rev: number };
  manifest!.files[relPath] = { hash: await hashOf(content), rev };
}

/** Uploads everything queued. Only `syncNow` calls this, so it never overlaps a pull. */
async function flush(): Promise<void> {
  if (!root || !manifest || !isSignedIn() || pending.size === 0) return;
  const batch = [...pending];
  pending.clear();
  setStatus({ state: "syncing" });

  const failed: string[] = [];
  let storageFull = false;
  for (const relPath of batch) {
    try {
      await pushOne(relPath);
    } catch (err) {
      console.warn("Sync push failed for", relPath, err);
      failed.push(relPath);
      if (err instanceof StorageFullError) storageFull = true;
    }
  }
  // A failed upload stays queued and rides along with the next attempt. Nothing
  // needs recovering — disk already holds the content.
  for (const relPath of failed) pending.add(relPath);

  await saveManifest();
  setStatus(
    failed.length > 0
      ? { state: "error", error: describeFailedPush(failed.length, storageFull) }
      : { state: "idle", lastSyncedAt: Date.now(), error: null },
  );
}

/* --- pull --- */

async function writeConflictCopy(relPath: string, content: string): Promise<void> {
  isApplyingRemote = true;
  try {
    let candidate = conflictName(relPath, new Date());
    for (let n = 1; n < 50 && (await pathExists(toAbsPath(root!, candidate))); n++) {
      candidate = conflictName(relPath, new Date(), n);
    }
    const abs = toAbsPath(root!, candidate);
    await ensureFolder(parentOf(abs));
    await writeTextFile(abs, content);
  } finally {
    isApplyingRemote = false;
  }
}

async function fetchContent(relPath: string): Promise<string> {
  const res = await authFetch(`/api/notes/content?path=${encodeURIComponent(relPath)}`);
  if (!res.ok) throw new Error(`Download of ${relPath} failed (${res.status})`);
  return ((await res.json()) as { content: string }).content;
}

/** Every file in the synced folder, as absolute paths. Dotfiles are skipped for us. */
function flatten(nodes: TreeNode[]): string[] {
  return nodes.flatMap((n) => (n.isFolder ? flatten(n.children ?? []) : [n.id]));
}

/**
 * Queues whatever changed on disk while the app was not the one writing.
 *
 * The push queue is otherwise fed only by this app's own saves, so a note edited
 * in another editor — or added to the folder by hand, or removed from it — never
 * reached the server at all, and the next pull read it as a conflict.
 *
 * Adds to `pending` directly rather than through `queue`: the `flush` at the end
 * of this same run takes them, and arming the debounce here would schedule
 * another run 2s later — which, with the server unreachable, is a scan of the
 * whole folder every two seconds for as long as the machine stays offline.
 *
 * ponytail: hashes every note on every pull. Fine for a notebook; if that gets
 * slow, compare a cheap stat first and hash only what looks different.
 */
async function scanLocal(): Promise<void> {
  const seen = new Set<string>();

  for (const abs of flatten(await readDirRecursive(root!))) {
    const rel = toRelPath(root!, abs);
    if (!rel || !isMarkdownFile(rel)) continue;
    seen.add(rel);
    if (manifest!.files[rel]?.hash !== (await hashOf(await readTextFile(abs)))) pending.add(rel);
  }

  // A file that is gone but still in the manifest was deleted behind our back.
  // Entries with no content of their own are tombstones, and already agree.
  for (const [rel, file] of Object.entries(manifest!.files)) {
    if (!seen.has(rel) && file.hash !== UNSYNCED) pending.add(rel);
  }
}

export async function pull(): Promise<void> {
  if (!root || !manifest || !isSignedIn()) return;
  setStatus({ state: "syncing" });
  await scanLocal();

  const res = await authFetch(`/api/notes?since=${manifest.cursor}`);
  if (!res.ok) throw new Error(`Sync list failed (${res.status})`);
  const { notes } = (await res.json()) as {
    notes: { relPath: string; rev: number; deletedAt: string | null }[];
  };

  const changed: string[] = [];

  for (const note of notes) {
    const known = manifest.files[note.relPath];

    // A device's own uploads and deletes come back in this list, because the
    // cursor only moves on a pull. Recognising the revision we already hold is
    // what stops an edit made since from being read as a remote change — which
    // filed a conflict copy of the device's own work on every second save.
    if (known?.rev === note.rev) {
      manifest.cursor = Math.max(manifest.cursor, note.rev);
      continue;
    }

    const abs = toAbsPath(root, note.relPath);
    const localExists = await pathExists(abs);
    const localHash = localExists ? await hashOf(await readTextFile(abs)) : null;

    const action = decidePull({
      remoteDeleted: note.deletedAt !== null,
      localExists,
      localHash,
      knownHash: known?.hash ?? null,
    });

    // The flag is held around the writes only. Holding it across a download let
    // a save the user made in that window be dropped instead of queued.
    if (action === "download") {
      const content = await fetchContent(note.relPath);
      isApplyingRemote = true;
      try {
        await ensureFolder(parentOf(abs));
        await writeTextFile(abs, content);
      } finally {
        isApplyingRemote = false;
      }
      manifest.files[note.relPath] = { hash: await hashOf(content), rev: note.rev };
      changed.push(abs);
    } else if (action === "delete") {
      isApplyingRemote = true;
      try {
        await deletePath(abs, false);
      } finally {
        isApplyingRemote = false;
      }
      manifest.files[note.relPath] = { hash: UNSYNCED, rev: note.rev };
      changed.push(abs);
    } else if (action === "conflict" || action === "keep-local") {
      if (action === "conflict") {
        await writeConflictCopy(note.relPath, await fetchContent(note.relPath));
        changed.push(abs);
      }
      // Local is still the newer edit, so re-upload it on top of this revision.
      // Recording the rev is what makes that retry land: starting from 0 instead
      // collided with the very row we just read and filed a second conflict copy.
      manifest.files[note.relPath] = { hash: UNSYNCED, rev: note.rev };
      pending.add(note.relPath);
    }

    manifest.cursor = Math.max(manifest.cursor, note.rev);
  }

  await saveManifest();
  setStatus({ state: "idle", lastSyncedAt: Date.now(), error: null });
  if (changed.length > 0) for (const cb of pulledListeners) cb(changed);
}

/** One pull followed by whatever is queued, never overlapping itself. */
export function syncNow(): Promise<void> {
  if (inFlight) return inFlight;
  inFlight = (async () => {
    try {
      await pull();
      await flush();
    } catch (err) {
      console.warn("Sync failed:", err);
      setStatus({ state: "error", error: err instanceof Error ? err.message : String(err) });
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

function onFocus(): void {
  void syncNow();
}

/**
 * Points sync at a folder for a user. Safe to call again when either changes;
 * calling with a null folder or while signed out just stops it.
 */
export async function startSync(folder: string | null, userId: string | null): Promise<void> {
  stopSync();
  if (!folder || !userId || !isSignedIn()) return;
  const mine = generation;

  root = folder;

  // The manifest is read inside the lock: a window that waited its turn must not
  // start from the bookkeeping as it stood before the other one was writing it.
  const begin = async () => {
    const loaded = await loadManifest(userId);
    if (generation !== mine) return;

    manifest = loaded;
    setLocalChangeListener(handleLocalChange);
    setStatus({ state: "idle", error: null });

    window.addEventListener("focus", onFocus);
    pullTimer = setInterval(() => void syncNow(), PULL_INTERVAL_MS);
    void syncNow();
  };

  // No Web Locks — an older browser, or the tests. Then this window is the only one.
  if (!navigator.locks) {
    await begin();
    return;
  }

  const wait = new AbortController();
  lockWait = wait;
  // Replaced by `begin` the moment the lock is ours, which is immediately unless
  // another tab is already syncing this folder.
  setStatus({ state: "waiting", error: null });
  void navigator.locks
    .request("zyplus:sync", { signal: wait.signal }, async () => {
      if (generation !== mine) return; // stopped while waiting in line
      await begin();
      if (generation !== mine) return; // stopped while starting up
      return new Promise<void>((release) => {
        releaseSyncLock = release;
      });
    })
    .catch(() => {
      // `stopSync` aborted the wait, which rejects the request. Nothing to undo.
    });
}

export function stopSync(): void {
  generation++;
  // One of the two applies: the lock is held, or this window is still in line.
  releaseSyncLock?.();
  releaseSyncLock = null;
  lockWait?.abort();
  lockWait = null;
  setLocalChangeListener(null);
  window.removeEventListener("focus", onFocus);
  if (pushTimer) clearTimeout(pushTimer);
  if (pullTimer) clearInterval(pullTimer);
  pushTimer = null;
  pullTimer = null;
  root = null;
  manifest = null;
  pending = new Set();
  setStatus({ state: "off", lastSyncedAt: null, error: null });
}
