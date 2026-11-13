/** Cross-component commands, fired on `window` so whichever component owns the UI can serve them. */
export const FIND_EVENT = "zyplus:find";
export const SETTINGS_EVENT = "zyplus:settings";
export const CLOSE_TAB_EVENT = "zyplus:close-tab";
/** Asks whichever editor is showing to take keyboard focus. */
export const FOCUS_EDITOR_EVENT = "zyplus:focus-editor";

export function emit(name: string, detail?: unknown) {
  window.dispatchEvent(new CustomEvent(name, { detail }));
}

/**
 * Puts the caret in the editor now and again shortly after. A menu, dialog or button that has just
 * done its job restores focus to whatever opened it as it closes, which can land after the editor
 * mounted and took focus; the second request wins that race. Focusing an editor that already has
 * focus does nothing, so the repeat is harmless.
 */
export function focusEditorSoon(): void {
  emit(FOCUS_EDITOR_EVENT);
  for (const ms of [80, 300]) window.setTimeout(() => emit(FOCUS_EDITOR_EVENT), ms);
}
