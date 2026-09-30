/**
 * `localStorage`, or null when the browser will not give it out. Merely reading
 * `window.localStorage` throws a SecurityError when site data is blocked, so the
 * old `!window.localStorage` guards crashed exactly where they meant to protect —
 * and `applyTheme()` runs at module load, before anything could catch it.
 */
export function safeStorage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : (window.localStorage ?? null);
  } catch {
    return null;
  }
}
