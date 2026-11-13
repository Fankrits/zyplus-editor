/**
 * A tiny registry between an editor that batches its change reports and the store that
 * reads document text.
 *
 * The source editor reports a big document's edits at most every few milliseconds instead of
 * on every keystroke (copying a megabyte out of CodeMirror per key is most of its cost). While
 * a batch is waiting, the store's copy of the text is a moment behind what is on screen, so
 * anything that reads the store to save, export, copy or close first asks the editor to
 * report now. It lives in its own module so the store can call it without importing an editor,
 * which would pull CodeMirror onto the startup path.
 */
interface Holder {
  flush: () => void;
  hasPending: () => boolean;
}
let holder: Holder | null = null;

/** The mounted editor's way to report its batched edits at once. Returns the way to unregister. */
export function registerPendingEdits(next: Holder): () => void {
  holder = next;
  return () => {
    if (holder === next) holder = null;
  };
}

/** True while an editor holds edits the store has not seen. */
export function hasPendingEdits(): boolean {
  return holder?.hasPending() ?? false;
}

/** Makes the store current with any edits an editor is still holding. */
export function flushPendingEdits(): void {
  holder?.flush();
}
