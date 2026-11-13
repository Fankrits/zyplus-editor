/**
 * Reports a document's text to the store, per change for small documents and in batches for
 * big ones.
 *
 * Reading the text out of CodeMirror copies all of it. Per keystroke that is free for a note
 * and most of the work for a multi-megabyte file (a fresh string each time, which the store
 * then compares to the saved one). Past `LARGE_DOC_CHARS` the report is throttled to one per
 * `BATCH_MS`, so a typing burst costs one copy per interval, and `flush` reports at once for
 * whoever needs the text now (see lib/pendingEdits.ts).
 */
export const LARGE_DOC_CHARS = 200_000;
export const BATCH_MS = 150;

/** Anything that can copy its current document text out: a CodeMirror `EditorView`. */
export interface TextSource {
  readonly state: { readonly doc: { toString(): string } };
}

export interface ChangeReporter {
  /**
   * The document changed. The text is copied out of `source` only when a report is due, and
   * `source` (the view) is all that is kept while one waits, not the update that caused it.
   */
  docChanged(length: number, source: TextSource): void;
  /** Reports a waiting batch now. A no-op when there is none. */
  flush(): void;
  hasPending(): boolean;
}

export function createChangeReporter(report: (text: string) => void): ChangeReporter {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: TextSource | null = null;

  const flush = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    const source = pending;
    pending = null;
    if (source) report(source.state.doc.toString());
  };

  return {
    docChanged(length, source) {
      if (length <= LARGE_DOC_CHARS) {
        // The document shrank back under the limit with a batch waiting: that batch is stale.
        if (timer !== undefined) clearTimeout(timer);
        timer = undefined;
        pending = null;
        report(source.state.doc.toString());
        return;
      }
      pending = source;
      // Throttle, not debounce: continuous typing still reports every interval.
      timer ??= setTimeout(flush, BATCH_MS);
    },
    flush,
    hasPending: () => pending !== null,
  };
}
