/**
 * How big a file Zyplus will open, and in which editor.
 *
 * The rich editor builds a ProseMirror document and a DOM node per block, which costs many
 * times the file's size in memory and a long layout pass; the source editor is far cheaper
 * per byte. Without a guard, opening one large file was the single biggest way to blow the
 * app's memory and freeze it. Sizes are bytes on disk (the web build counts the UTF-8 length).
 */

/**
 * Above this a file opens in the source editor; the user can still switch to rich. Measured
 * (`bun run perf:bigfile`): rich typing costs ~15 ms a key at 60 KB, ~28 at 200 KB, ~41 at 300 KB and
 * ~360 at 1 MB, against ~13 ms at 1 MB in the source editor. 200 KB is where a key stops fitting in
 * two frames.
 */
export const RICH_TEXT_LIMIT = 200 * 1024;

/** Above this a file is not opened at all. */
export const MAX_OPEN_SIZE = 20 * 1024 * 1024;

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function isTooLargeToOpen(bytes: number): boolean {
  return bytes > MAX_OPEN_SIZE;
}

/** The editor a file of this size should start in: `wanted`, unless it is too big for rich. */
export function modeForSize<M extends string>(bytes: number, wanted: M): M | "plain" {
  return wanted === "rich" && bytes > RICH_TEXT_LIMIT ? "plain" : wanted;
}

export function tooLargeMessage(bytes: number): string {
  return (
    `This file is ${formatSize(bytes)}. Zyplus opens files up to ${formatSize(MAX_OPEN_SIZE)}, ` +
    "because a larger one would use more memory than the editor can handle comfortably. " +
    "Open it in a dedicated text editor instead."
  );
}
