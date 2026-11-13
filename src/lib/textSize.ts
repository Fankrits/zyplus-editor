import { safeStorage } from "./safeStorage";

/**
 * How big the text in the editors is, as a percentage of the default. It scales the document text (both
 * the rich and the source editor, headings and code included) and leaves the app's own controls alone.
 *
 * The value lives in a CSS variable, `--editor-text-scale`, which the editors' stylesheets multiply
 * their base font size by, so changing it needs no re-render and no editor remount.
 */
export const TEXT_SIZES = [80, 90, 100, 110, 125, 150, 175, 200] as const;
export const DEFAULT_TEXT_SIZE = 100;

const KEY = "zyplus:text-size";
const VAR = "--editor-text-scale";

const isAllowed = (n: number): n is (typeof TEXT_SIZES)[number] => (TEXT_SIZES as readonly number[]).includes(n);

let current: number | null = null;
const listeners = new Set<() => void>();

/** The saved size; anything that is not one of the offered steps falls back to the default. */
export function getTextSize(): number {
  if (current !== null) return current;
  let stored: string | null = null;
  try {
    stored = safeStorage()?.getItem(KEY) ?? null;
  } catch {
    // Reading can still throw once storage is denied; the default is the fallback.
  }
  const n = stored === null ? NaN : Number(stored);
  current = isAllowed(n) ? n : DEFAULT_TEXT_SIZE;
  return current;
}

/** Writes the size onto the page. Safe to call before anything has rendered. */
export function applyTextSize(size: number = getTextSize()): void {
  if (typeof document === "undefined") return;
  document.documentElement.style.setProperty(VAR, String(size / 100));
}

/** Sets, saves and applies a size, snapping to the nearest offered step. Returns the size now in force. */
export function setTextSize(size: number): number {
  const next = TEXT_SIZES.reduce((best, s) => (Math.abs(s - size) < Math.abs(best - size) ? s : best), DEFAULT_TEXT_SIZE as number);
  if (next === getTextSize()) {
    applyTextSize(next);
    return next;
  }
  current = next;
  try {
    safeStorage()?.setItem(KEY, String(next));
  } catch {
    // Ignore quota/private-mode errors; the size still applies for this session.
  }
  applyTextSize(next);
  for (const listener of listeners) listener();
  return next;
}

/** One step bigger (`1`) or smaller (`-1`), stopping at the ends. */
export function stepTextSize(direction: 1 | -1): number {
  const i = TEXT_SIZES.indexOf(getTextSize() as (typeof TEXT_SIZES)[number]);
  const next = TEXT_SIZES[Math.min(TEXT_SIZES.length - 1, Math.max(0, i + direction))];
  return setTextSize(next);
}

export function resetTextSize(): number {
  return setTextSize(DEFAULT_TEXT_SIZE);
}

export const canIncreaseTextSize = () => getTextSize() < TEXT_SIZES[TEXT_SIZES.length - 1];
export const canDecreaseTextSize = () => getTextSize() > TEXT_SIZES[0];

/** For `useSyncExternalStore`: the settings screen follows the keyboard shortcuts and vice versa. */
export function subscribeTextSize(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Forgets the cached value, so the next read goes back to storage. For tests. */
export function resetTextSizeCache(): void {
  current = null;
}
