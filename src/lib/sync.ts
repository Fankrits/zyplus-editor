import { useSyncExternalStore } from "react";
import { authFetch, isSignedIn } from "./auth";
import {
  deletePath,
  ensureFolder,
  isMarkdownFile,
  pathExists,
  readTextFile,
  setLocalChangeListener,
  writeTextFile,
} from "./fs";

export const MANIFEST_DIR = ".zyplus";
export const MANIFEST_FILE = "sync.json";

const PUSH_DEBOUNCE_MS = 2000;
const PULL_INTERVAL_MS = 5 * 60 * 1000;

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

export interface SyncStatus {
  state: "off" | "idle" | "syncing" | "error";
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
  isApplyingRemote = true;
  try {
    await ensureFolder(toAbsPath(root, MANIFEST_DIR));
    await writeTextFile(manifestPath(), JSON.stringify(manifest, null, 2));
  } catch (err) {
    console.warn("Failed to write the sync manifest:", err);
  } finally {
    isApplyingRemote = false;
  }
}

/* --- push --- */

function queue(relPath: string): void {
  pending.add(relPath);
  if (pushTimer) clearTimeout(pushTimer);
  pushTimer = setTimeout(() => void flush(), PUSH_DEBOUNCE_MS);
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
    await authFetch(`/api/notes?path=${encodeURIComponent(relPath)}`, { method: "DELETE" });
    delete manifest!.files[relPath];
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

  if (!res.ok) throw new Error(`Upload of ${relPath} failed (${res.status})`);
  const { rev } = (await res.json()) as { rev: number };
  manifest!.files[relPath] = { hash: await hashOf(content), rev };
}

async function flush(): Promise<void> {
  if (!root || !manifest || !isSignedIn() || pending.size === 0) return;
  const batch = [...pending];
  pending.clear();
  setStatus({ state: "syncing" });

  const failed: string[] = [];
  for (const relPath of batch) {
    try {
      await pushOne(relPath);
    } catch (err) {
      console.warn("Sync push failed for", relPath, err);
      failed.push(relPath);
    }
  }
  // A failed upload stays queued and rides along with the next attempt. Nothing
  // needs recovering — disk already holds the content.
  for (const relPath of failed) pending.add(relPath);

  await saveManifest();
  setStatus(
    failed.length > 0
      ? { state: "error", error: "Some changes are waiting to upload" }
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

export async function pull(): Promise<void> {
  if (!root || !manifest || !isSignedIn()) return;
  setStatus({ state: "syncing" });

  const res = await authFetch(`/api/notes?since=${manifest.cursor}`);
  if (!res.ok) throw new Error(`Sync list failed (${res.status})`);
  const { notes } = (await res.json()) as {
    notes: { relPath: string; rev: number; deletedAt: string | null }[];
  };

  const changed: string[] = [];

  for (const note of notes) {
    const abs = toAbsPath(root, note.relPath);
    const localExists = await pathExists(abs);
    const localHash = localExists ? await hashOf(await readTextFile(abs)) : null;
    const known = manifest.files[note.relPath];

    const action = decidePull({
      remoteDeleted: note.deletedAt !== null,
      localExists,
      localHash,
      knownHash: known?.hash ?? null,
    });

    isApplyingRemote = true;
    try {
      if (action === "download") {
        const content = await fetchContent(note.relPath);
        await ensureFolder(parentOf(abs));
        await writeTextFile(abs, content);
        manifest.files[note.relPath] = { hash: await hashOf(content), rev: note.rev };
        changed.push(abs);
      } else if (action === "delete") {
        await deletePath(abs, false);
        delete manifest.files[note.relPath];
        changed.push(abs);
      } else if (action === "conflict") {
        const content = await fetchContent(note.relPath);
        isApplyingRemote = false;
        await writeConflictCopy(note.relPath, content);
        isApplyingRemote = true;
        // Local is still the newer edit, so re-upload it on top of this revision.
        delete manifest.files[note.relPath];
        queue(note.relPath);
        changed.push(abs);
      } else if (action === "keep-local") {
        delete manifest.files[note.relPath];
        queue(note.relPath);
      }
    } finally {
      isApplyingRemote = false;
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
  const loaded = await loadManifest(userId);
  if (generation !== mine) return;

  manifest = loaded;
  setLocalChangeListener(handleLocalChange);
  setStatus({ state: "idle", error: null });

  window.addEventListener("focus", onFocus);
  pullTimer = setInterval(() => void syncNow(), PULL_INTERVAL_MS);
  void syncNow();
}

export function stopSync(): void {
  generation++;
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
