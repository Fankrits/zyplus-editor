/**
 * Copies text, and says so if it could not. `navigator.clipboard` is missing on
 * insecure origins and rejects when the page lacks permission; every caller is a
 * menu item or shortcut that used to drop that rejection, so "Copy" silently did
 * nothing.
 */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch (err) {
    console.error("Could not copy to the clipboard:", err);
    try {
      window.alert("Could not copy to the clipboard. Your browser or system blocked the request.");
    } catch {
      // No window to alert in; the console line is the record.
    }
  }
}
