export type Theme = keyof typeof THEMES;
type Base = "light" | "dark";

/**
 * `base` picks the HeroUI theme the palette sits on (it supplies every token a
 * palette does not override, plus Tailwind's `dark:` variant); `swatch` is
 * [background, surface, accent], only for the settings preview.
 */
export const THEMES = {
  system: { label: "System", group: "Base", base: null, swatch: ["#ffffff", "#000000", "#006fee"] },
  light: { label: "Light", group: "Base", base: "light", swatch: ["#ffffff", "#f1f1f4", "#006fee"] },
  dark: { label: "Dark", group: "Base", base: "dark", swatch: ["#18181b", "#3f3f46", "#63b3ff"] },

  latte: { label: "Latte", group: "Catppuccin", base: "light", swatch: ["#eff1f5", "#ccd0da", "#1e66f5"] },
  frappe: { label: "Frappé", group: "Catppuccin", base: "dark", swatch: ["#303446", "#414559", "#8caaee"] },
  macchiato: { label: "Macchiato", group: "Catppuccin", base: "dark", swatch: ["#24273a", "#363a4f", "#8aadf4"] },
  mocha: { label: "Mocha", group: "Catppuccin", base: "dark", swatch: ["#1e1e2e", "#313244", "#89b4fa"] },

  "tokyo-night": { label: "Night", group: "Tokyo Night", base: "dark", swatch: ["#1a1b26", "#292e42", "#7aa2f7"] },
  "tokyo-storm": { label: "Storm", group: "Tokyo Night", base: "dark", swatch: ["#24283b", "#2f334d", "#7aa2f7"] },
  "tokyo-day": { label: "Day", group: "Tokyo Night", base: "light", swatch: ["#e1e2e7", "#c4c8da", "#2e7de9"] },
} as const satisfies Record<string, { label: string; group: string; base: Base | null; swatch: readonly string[] }>;

const THEME_KEY = "zyplus:theme";
/** index.html resolves the theme before first paint and has no access to THEMES. */
const BASE_KEY = "zyplus:theme-base";

export function getTheme(): Theme {
  if (typeof window === "undefined" || !window.localStorage) return "system";
  const stored = localStorage.getItem(THEME_KEY);
  return stored && stored in THEMES ? (stored as Theme) : "system";
}

/** HeroUI keys both its CSS variables and the `dark:` variant off `data-theme`. */
export function applyTheme(theme: Theme = getTheme()): void {
  if (typeof document === "undefined") return;
  const base =
    THEMES[theme].base ??
    (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  const root = document.documentElement;
  root.dataset.theme = base;
  if (THEMES[theme].base === null) delete root.dataset.palette;
  else root.dataset.palette = theme;
  // index.html sets this inline before paint; keep it in sync so native widgets follow.
  root.style.colorScheme = base;
}

export function setTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
    localStorage.setItem(BASE_KEY, THEMES[theme].base ?? "");
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
