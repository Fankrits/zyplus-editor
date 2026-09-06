/** Cross-component commands, fired on `window` so whichever component owns the UI can serve them. */
export const FIND_EVENT = "zyplus:find";
export const SETTINGS_EVENT = "zyplus:settings";
export const CLOSE_TAB_EVENT = "zyplus:close-tab";

export function emit(name: string, detail?: unknown) {
  window.dispatchEvent(new CustomEvent(name, { detail }));
}
