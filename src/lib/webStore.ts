/**
 * The web build's disk: every file and folder it holds, in IndexedDB, keyed by
 * path. A file's value is its content; a folder's is null. `lib/fs.ts` keeps
 * the working copy in memory and writes through here before touching it, so a
 * write the browser refuses — out of quota, say — never looks like it landed.
 *
 * Each tab holds its own copy of that working set, so every write is broadcast
 * to the others and applied there too. Without it, two open tabs each wrote
 * through a copy that had stopped matching the database, and whichever one saved
 * last silently threw away the other's notes.
 */

const DB_NAME = "zyplus";
const STORE = "entries";

/** A write to apply: content, null for a folder, undefined to delete. */
export type WebOp = [path: string, value: string | null | undefined];

let db: IDBDatabase | null = null;

/** Absent in workers and older browsers; then a second tab is simply on its own. */
const channel = typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel("zyplus:fs");

/** Applies another tab's writes. One listener — `lib/fs.ts` owns the working copy. */
export function onOtherTabWrite(cb: (ops: WebOp[]) => void): void {
  if (channel) channel.onmessage = (event) => cb(event.data as WebOp[]);
}

function done<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

/** Opens the store and returns everything in it. */
export async function openWebStore(): Promise<[string, string | null][]> {
  const open = indexedDB.open(DB_NAME, 1);
  open.onupgradeneeded = () => open.result.createObjectStore(STORE);
  db = await done(open);
  // Without this the browser may evict the notes under storage pressure.
  void navigator.storage?.persist?.();

  const store = db.transaction(STORE).objectStore(STORE);
  const [keys, values] = await Promise.all([done(store.getAllKeys()), done(store.getAll())]);
  return keys.map((key, i) => [String(key), values[i] as string | null]);
}

/** Applies every op in one transaction. A no-op until `openWebStore` has run (tests). */
export function persistWeb(ops: WebOp[]): Promise<void> {
  if (!db || ops.length === 0) return Promise.resolve();
  const tx = db.transaction(STORE, "readwrite");
  const store = tx.objectStore(STORE);
  for (const [path, value] of ops) {
    if (value === undefined) store.delete(path);
    else store.put(value, path);
  }
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => {
      // Only after the database took it: a tab must never show a write that failed.
      channel?.postMessage(ops);
      resolve();
    };
    tx.onerror = tx.onabort = () => reject(tx.error);
  });
}
