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

export interface ChangeReporter {
  /** The document changed; `read` copies its current text out, and is only called when a report is due. */
  docChanged(length: number, read: () => string): void;
  /** Reports a waiting batch now. A no-op when there is none. */
  flush(): void;
  hasPending(): boolean;
}

export function createChangeReporter(report: (text: string) => void): ChangeReporter {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pendingRead: (() => string) | null = null;

  const flush = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    const read = pendingRead;
    pendingRead = null;
    if (read) report(read());
  };

  return {
    docChanged(length, read) {
      if (length <= LARGE_DOC_CHARS) {
        // The document shrank back under the limit with a batch waiting: that batch is stale.
        if (timer !== undefined) clearTimeout(timer);
        timer = undefined;
        pendingRead = null;
        report(read());
        return;
      }
      pendingRead = read;
      // Throttle, not debounce: continuous typing still reports every interval.
      timer ??= setTimeout(flush, BATCH_MS);
    },
    flush,
    hasPending: () => pendingRead !== null,
  };
}
