/**
 * The web build's disk: every file and folder it holds, in IndexedDB, keyed by
 * path. A file's value is its content; a folder's is null. `lib/fs.ts` keeps
 * the working copy in memory and writes through here before touching it, so a
 * write the browser refuses — out of quota, say — never looks like it landed.
 *
 * ponytail: each browser tab loads its own copy once, so two tabs editing at the
 * same time overwrite each other. A BroadcastChannel reload would fix that.
 */

const DB_NAME = "zyplus";
const STORE = "entries";

/** A write to apply: content, null for a folder, undefined to delete. */
export type WebOp = [path: string, value: string | null | undefined];

let db: IDBDatabase | null = null;

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
    tx.oncomplete = () => resolve();
    tx.onerror = tx.onabort = () => reject(tx.error);
  });
}
