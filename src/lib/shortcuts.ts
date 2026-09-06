/**
 * Single source of truth for keyboard shortcuts: App binds `id` to an action,
 * the shortcuts sheet renders the same table.
 *
 * Combo syntax: "mod+shift+alt+<key>", where `mod` is ⌘ on macOS and Ctrl elsewhere.
 * Keys are matched on `event.code` (layout position) so Alt-modified keys still match
 * on macOS, where `event.key` becomes "ƒ" for Alt+F.
 */
export type CommandId =
  | "save"
  | "export-md"
  | "export-pdf"
  | "new-file"
  | "new-folder"
  | "open-folder"
  | "open-file"
  | "close-tab"
  | "next-tab"
  | "prev-tab"
  | "toggle-sidebar"
  | "toggle-mode"
  | "settings"
  | "shortcuts"
  | "find"
  | "replace"
  | "copy-markdown"
  | "copy-path";

export interface Shortcut {
  id: CommandId;
  /** First combo is the one shown in the sheet; the rest are aliases. */
  combos: string[];
  label: string;
  group: "File" | "Tabs" | "View" | "Editing";
}

export const SHORTCUTS: Shortcut[] = [
  { id: "save", combos: ["mod+s"], label: "Save", group: "File" },
  { id: "export-md", combos: ["mod+shift+s"], label: "Export as .md", group: "File" },
  { id: "export-pdf", combos: ["mod+p"], label: "Export as PDF", group: "File" },
  { id: "new-file", combos: ["mod+n"], label: "New file", group: "File" },
  { id: "new-folder", combos: ["mod+shift+n"], label: "New folder", group: "File" },
  { id: "open-folder", combos: ["mod+o"], label: "Open folder", group: "File" },
  { id: "open-file", combos: ["mod+shift+o"], label: "Open file", group: "File" },

  { id: "close-tab", combos: ["mod+w"], label: "Close tab", group: "Tabs" },
  { id: "next-tab", combos: ["mod+alt+arrowright", "mod+shift+bracketright"], label: "Next tab", group: "Tabs" },
  { id: "prev-tab", combos: ["mod+alt+arrowleft", "mod+shift+bracketleft"], label: "Previous tab", group: "Tabs" },

  { id: "toggle-sidebar", combos: ["mod+b"], label: "Toggle sidebar", group: "View" },
  { id: "toggle-mode", combos: ["mod+e"], label: "Toggle rich / plain text", group: "View" },
  { id: "settings", combos: ["mod+,"], label: "Settings", group: "View" },
  { id: "shortcuts", combos: ["mod+/"], label: "Keyboard shortcuts", group: "View" },

  { id: "find", combos: ["mod+f"], label: "Find in document", group: "Editing" },
  { id: "replace", combos: ["mod+alt+f"], label: "Find and replace", group: "Editing" },
  { id: "copy-markdown", combos: ["mod+shift+c"], label: "Copy document markdown", group: "Editing" },
  { id: "copy-path", combos: ["mod+alt+c"], label: "Copy file path", group: "Editing" },
];

/** Cmd+1…9 jump to the nth tab (9 = last). Listed in the sheet as one row. */
export const TAB_DIGIT_LABEL = "Go to tab 1–9";

export const isMac =
  typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.userAgent);

const CODE_ALIASES: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  BracketLeft: "[",
  BracketRight: "]",
  Minus: "-",
  Equal: "=",
  Space: "space",
};

/** Layout-position name of the pressed key, lowercased. */
export function keyOf(e: KeyboardEvent): string {
  const { code } = e;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (code in CODE_ALIASES) return CODE_ALIASES[code];
  if (code.startsWith("Arrow") || code === "Escape" || code === "Enter") return code.toLowerCase();
  return e.key.toLowerCase();
}

export function matchesCombo(e: KeyboardEvent, combo: string): boolean {
  const parts = combo.split("+");
  const key = parts[parts.length - 1];
  const mods = parts.slice(0, -1);
  if (mods.includes("mod") !== (isMac ? e.metaKey : e.ctrlKey)) return false;
  if (mods.includes("shift") !== e.shiftKey) return false;
  if (mods.includes("alt") !== e.altKey) return false;
  // On macOS the non-`mod` control key must stay clear so Ctrl-based bindings don't double-fire.
  if (isMac && e.ctrlKey) return false;
  if (!isMac && e.metaKey) return false;
  return keyOf(e) === key;
}

export function matchShortcut(e: KeyboardEvent): CommandId | null {
  for (const s of SHORTCUTS) {
    if (s.combos.some((c) => matchesCombo(e, c))) return s.id;
  }
  return null;
}

const DISPLAY: Record<string, string> = isMac
  ? { mod: "⌘", shift: "⇧", alt: "⌥", arrowleft: "←", arrowright: "→" }
  : { mod: "Ctrl", shift: "Shift", alt: "Alt", arrowleft: "←", arrowright: "→" };

/** "mod+shift+s" → ["⌘", "⇧", "S"] */
export function formatCombo(combo: string): string[] {
  return combo.split("+").map((p) => DISPLAY[p] ?? (p.length === 1 ? p.toUpperCase() : p));
}
