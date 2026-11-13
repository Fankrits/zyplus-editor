/**
 * True when the user is typing in a text field that is not the editor (a rename box in the file tree, a
 * dialog's input, the find bar). An editor that grabs focus on its own must not take it from there:
 * a rename box that loses focus cancels the rename.
 */
export function isTypingElsewhere(editorRoot: Element | null): boolean {
  const active = document.activeElement as HTMLElement | null;
  if (!active || active === document.body) return false;
  if (editorRoot?.contains(active)) return false;
  return (
    active instanceof HTMLInputElement ||
    active instanceof HTMLTextAreaElement ||
    active instanceof HTMLSelectElement ||
    active.isContentEditable
  );
}
