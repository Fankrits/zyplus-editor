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
 * How often the folder is re-hashed to find edits made outside the app. The pull
 * itself is one small request, but the scan reads and hashes every note, so at
 * the pull interval it would be the app's biggest idle cost. Coming back to the
 * window resets it, which is when an outside edit has actually just happened.
 */
const SCAN_INTERVAL_MS = 5 * 60 * 1000;

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

export type PullAction = "download" | "adopt" | "conflict" | "delete" | "keep-local" | "none";

/**
 * What to do with one changed remote note. Cloud-first: the cloud copy is the
 * note, and a diverged local edit is what moves aside.
 *
 * The whole thing turns on two questions. Does disk already hold exactly what
 * the cloud has? Then there is nothing to reconcile, whatever the manifest says
 * — a lost manifest or a folder copied to a new machine used to file a conflict
 * copy of every note. Otherwise, is the local file still what the server last
 * gave us? If it is, the remote copy is strictly newer and can be applied. If it
 * is not, both sides moved: the cloud copy takes the note's name and the local
 * edit is kept beside it.
 */
export function decidePull(args: {
  remoteDeleted: boolean;
  localExists: boolean;
  /** Hash of the file on disk now; null when it is not there. */
  localHash: string | null;
  /** Hash recorded at the last successful sync; null when never synced. */
  knownHash: string | null;
  /** Hash of the cloud content; null for a tombstone. */
  remoteHash: string | null;
}): PullAction {
  const { remoteDeleted, localExists, localHash, knownHash, remoteHash } = args;
  const localUntouched = localHash !== null && localHash === knownHash;

  if (remoteDeleted) {
    if (!localExists) return "none";
    return localUntouched ? "delete" : "keep-local";
  }
  if (!localExists) return "download";
  if (localHash === remoteHash) return "adopt";
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
let inFlightGeneration = -1;
let lastScanAt = 0;

/**
 * Bumped by every `startSync`/`stopSync`. `startSync` awaits the manifest, and
 * in that gap the folder or account can change — under StrictMode it always
 * does — so a run that is no longer current has to drop out rather than install
 * a second timer over the live one.
 */
let generation = 0;

/** An upload the server refused because the account is out of storage. */
class StorageFullError extends Error {}

/** Thrown into a run that `stopSync` cancelled while it was awaiting. */
class SyncStopped extends Error {}

/**
 * Called after every await in a run. `stopSync` nulls the folder and manifest
 * underneath it, and a quick `startSync` installs another account's — so a
 * cancelled run has to stop before it touches either, not crash into them.
 */
function ensureCurrent(mine: number): void {
  if (generation !== mine) throw new SyncStopped();
}

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

/**
 * Writes made by sync itself must not bounce straight back as uploads, so the
 * change listener stands down while remote content is being applied.
 */
let isApplyingRemote = false;

/**
 * Runs `write` with the change listener stood down. It wraps the writes alone:
 * holding the flag across a download let a save the user made in that window be
 * dropped instead of queued.
 */
async function applyingRemote<T>(write: () => Promise<T>): Promise<T> {
  isApplyingRemote = true;
  try {
    return await write();
  } finally {
    isApplyingRemote = false;
  }
}

let status: SyncStatus = { state: "off", lastSyncedAt: null, error: null };
const statusListeners = new Set<(s: SyncStatus) => void>();
/** `replaced` maps a note the cloud overwrote to the copy holding its local edit. */
type PulledListener = (changedPaths: string[], replaced: Record<string, string>) => void;
const pulledListeners = new Set<PulledListener>();

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
export function onPulled(cb: PulledListener): () => void {
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

async function pushOne(relPath: string, mine: number): Promise<void> {
  const abs = toAbsPath(root!, relPath);
  const known = manifest!.files[relPath];

  if (!(await pathExists(abs))) {
    const res = await authFetch(`/api/notes?path=${encodeURIComponent(relPath)}`, {
      method: "DELETE",
    });
    if (!res.ok) throw new Error(`Delete of ${relPath} failed (${res.status})`);
    const { rev } = (await res.json()) as { rev: number };
    ensureCurrent(mine);
    // Remember the tombstone rather than forgetting the path. Re-creating a note
    // with the same name then uploads on top of the tombstone; forgetting it made
    // the upload start from rev 0, collide with the tombstone, and leave an empty
    // conflict copy beside the new note.
    manifest!.files[relPath] = { hash: UNSYNCED, rev };
    return;
  }

  const content = await readTextFile(abs);
  const hash = await hashOf(content);
  // Already what the cloud holds — a pull just adopted or downloaded it.
  if (known?.hash === hash) return;

  const res = await authFetch("/api/notes", {
    method: "PUT",
    body: JSON.stringify({ relPath, content, baseRev: known?.rev ?? 0 }),
  });

  if (res.status === 409) {
    // Another device wrote since our pull. The cloud wins, so this is settled by
    // the next pull like any other remote change — `decidePull` adopts it when
    // the content matches, and otherwise moves this edit aside. `queue` runs
    // that pull and then retries whatever is still left to upload.
    ensureCurrent(mine);
    queue(relPath);
    return;
  }

  if (res.status === 413) throw new StorageFullError(`No storage left for ${relPath}`);
  if (!res.ok) throw new Error(`Upload of ${relPath} failed (${res.status})`);
  const { rev } = (await res.json()) as { rev: number };
  ensureCurrent(mine);
  manifest!.files[relPath] = { hash, rev };
}

/** Uploads everything queued. Only `syncNow` calls this, so it never overlaps a pull. */
async function flush(): Promise<void> {
  if (!root || !manifest || !isSignedIn() || pending.size === 0) return;
  const mine = generation;
  const batch = [...pending];
  pending.clear();
  setStatus({ state: "syncing" });

  const failed: string[] = [];
  let storageFull = false;
  for (const relPath of batch) {
    try {
      await pushOne(relPath, mine);
    } catch (err) {
      if (err instanceof SyncStopped) throw err;
      console.warn("Sync push failed for", relPath, err);
      failed.push(relPath);
      if (err instanceof StorageFullError) storageFull = true;
    }
  }
  // A failed upload stays queued and rides along with the next attempt. Nothing
  // needs recovering — disk already holds the content.
  ensureCurrent(mine);
  for (const relPath of failed) pending.add(relPath);

  await saveManifest();
  setStatus(
    failed.length > 0
      ? { state: "error", error: describeFailedPush(failed.length, storageFull) }
      : { state: "idle", lastSyncedAt: Date.now(), error: null },
  );
}

/* --- pull --- */

/** Writes `content` beside `relPath` under a free conflict name, and returns that name. */
async function writeConflictCopy(relPath: string, content: string): Promise<string> {
  let candidate = conflictName(relPath, new Date());
  for (let n = 1; n < 50 && (await pathExists(toAbsPath(root!, candidate))); n++) {
    candidate = conflictName(relPath, new Date(), n);
  }
  const abs = toAbsPath(root!, candidate);
  await applyingRemote(async () => {
    await ensureFolder(parentOf(abs));
    await writeTextFile(abs, content);
  });
  return candidate;
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
  const mine = generation;
  const current = () => ensureCurrent(mine);
  setStatus({ state: "syncing" });
  if (Date.now() - lastScanAt >= SCAN_INTERVAL_MS) {
    await scanLocal();
    current();
    lastScanAt = Date.now();
  }

  const res = await authFetch(`/api/notes?since=${manifest!.cursor}`);
  if (!res.ok) throw new Error(`Sync list failed (${res.status})`);
  const { notes } = (await res.json()) as {
    notes: { relPath: string; rev: number; deletedAt: string | null }[];
  };
  current();
  // Past every `current()` the run is still the live one, so these are set.
  const manifestNow = manifest!;
  const rootNow = root!;

  const changed: string[] = [];
  const replaced: Record<string, string> = {};

  for (const note of notes) {
    const known = manifestNow.files[note.relPath];

    // A device's own uploads and deletes come back in this list, because the
    // cursor only moves on a pull. Recognising the revision we already hold is
    // what stops an edit made since from being read as a remote change — which
    // filed a conflict copy of the device's own work on every second save.
    if (known?.rev === note.rev) {
      manifestNow.cursor = Math.max(manifestNow.cursor, note.rev);
      continue;
    }

    const abs = toAbsPath(rootNow, note.relPath);
    const remoteDeleted = note.deletedAt !== null;
    // Fetched before disk is read, so no network wait sits between reading the
    // local file and overwriting it — a save landing in that gap was lost.
    const remote = remoteDeleted ? null : await fetchContent(note.relPath);
    const remoteHash = remote === null ? null : await hashOf(remote);
    current();
    const localExists = await pathExists(abs);
    const local = localExists ? await readTextFile(abs) : null;
    const localHash = local === null ? null : await hashOf(local);
    current();

    const action = decidePull({
      remoteDeleted,
      localExists,
      localHash,
      knownHash: known?.hash ?? null,
      remoteHash,
    });

    if (action === "adopt") {
      manifestNow.files[note.relPath] = { hash: remoteHash!, rev: note.rev };
    } else if (action === "download" || action === "conflict") {
      if (action === "conflict") {
        // Cloud-first: the local edit moves aside and goes up as its own note,
        // so the other devices see it too rather than it living only here.
        const copy = await writeConflictCopy(note.relPath, local!);
        current();
        pending.add(copy);
        const copyAbs = toAbsPath(rootNow, copy);
        replaced[abs] = copyAbs;
        changed.push(copyAbs);
      }
      await applyingRemote(async () => {
        await ensureFolder(parentOf(abs));
        await writeTextFile(abs, remote!);
      });
      manifestNow.files[note.relPath] = { hash: remoteHash!, rev: note.rev };
      changed.push(abs);
    } else if (action === "delete") {
      await applyingRemote(() => deletePath(abs, false));
      manifestNow.files[note.relPath] = { hash: UNSYNCED, rev: note.rev };
      changed.push(abs);
    } else if (action === "keep-local") {
      // Deleted in the cloud but edited here: the edit is not thrown away, so it
      // is re-uploaded on top of the tombstone. Recording the rev is what makes
      // that retry land: starting from 0 instead collided with the very row we
      // just read and filed a second conflict copy.
      manifestNow.files[note.relPath] = { hash: UNSYNCED, rev: note.rev };
      pending.add(note.relPath);
    }

    manifestNow.cursor = Math.max(manifestNow.cursor, note.rev);
  }

  current();
  await saveManifest();
  setStatus({ state: "idle", lastSyncedAt: Date.now(), error: null });
  if (changed.length > 0) for (const cb of pulledListeners) cb(changed, replaced);
}

/** One pull followed by whatever is queued, never overlapping itself. */
export function syncNow(): Promise<void> {
  // A run left over from before a restart is on its way out; it is not this one.
  if (inFlight && inFlightGeneration === generation) return inFlight;
  const mine = generation;
  const run = (async () => {
    try {
      await pull();
      await flush();
    } catch (err) {
      // Stopped mid-run: the status already says "off", or belongs to the next run.
      if (generation !== mine) return;
      console.warn("Sync failed:", err);
      setStatus({ state: "error", error: err instanceof Error ? err.message : String(err) });
    } finally {
      if (inFlightGeneration === mine) inFlight = null;
    }
  })();
  inFlight = run;
  inFlightGeneration = mine;
  return run;
}

function onFocus(): void {
  // Back in the window is exactly when someone has been editing elsewhere.
  lastScanAt = 0;
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

  // Replaced by `begin` the moment the lock is ours, which is immediately unless
  // another tab is already syncing this folder.
  setStatus({ state: "waiting", error: null });
  // A request this window no longer wants is left in the queue rather than
  // cancelled: when its turn comes the generation no longer matches, so it
  // returns at once and the lock passes straight to whoever is next in line.
  void navigator.locks
    .request("zyplus:sync", async () => {
      if (generation !== mine) return; // stopped while waiting in line
      await begin();
      if (generation !== mine) return; // stopped while starting up
      return new Promise<void>((release) => {
        releaseSyncLock = release;
      });
    })
    .catch((err) => console.warn("Could not take the sync lock:", err));
}

export function stopSync(): void {
  generation++;
  // The run in progress bails at its next await; nothing should wait on it.
  inFlight = null;
  releaseSyncLock?.();
  releaseSyncLock = null;
  setLocalChangeListener(null);
  window.removeEventListener("focus", onFocus);
  if (pushTimer) clearTimeout(pushTimer);
  if (pullTimer) clearInterval(pullTimer);
  pushTimer = null;
  pullTimer = null;
  root = null;
  manifest = null;
  pending = new Set();
  lastScanAt = 0;
  setStatus({ state: "off", lastSyncedAt: null, error: null });
}
