export type Theme = "system" | "light" | "dark";

const THEME_KEY = "zyplus:theme";

export function getTheme(): Theme {
  if (typeof window === "undefined" || !window.localStorage) return "system";
  const stored = localStorage.getItem(THEME_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

/** HeroUI keys both its CSS variables and the `dark:` variant off `data-theme`. */
export function applyTheme(theme: Theme = getTheme()): void {
  if (typeof document === "undefined") return;
  const resolved =
    theme === "system"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light"
      : theme;
  document.documentElement.dataset.theme = resolved;
  // index.html sets this inline before paint; keep it in sync so native widgets follow.
  document.documentElement.style.colorScheme = resolved;
}

export function setTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Ignore quota/private-mode errors; the theme still applies for this session.
  }
  applyTheme(theme);
}

if (typeof window !== "undefined" && window.matchMedia) {
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", () => getTheme() === "system" && applyTheme("system"));
}

/** Resolved theme, for the places (mermaid) that need a literal color, not a CSS variable. */
export function isDarkTheme(): boolean {
  return typeof document !== "undefined" && document.documentElement.dataset.theme === "dark";
}
