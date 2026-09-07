/**
 * Which OS the app is running on, as seen from the webview.
 *
 * Tauri ships the host's real user agent (WKWebView on macOS, WebView2 on
 * Windows, WebKitGTK on Linux), so this is the same check the browser build
 * makes and needs no extra plugin or IPC round trip.
 */
const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";

export const isMac = /mac|iphone|ipad/i.test(ua);
export const isWindows = /windows/i.test(ua);

/**
 * What the platform calls "show this file where it lives". Using the macOS
 * wording everywhere sends Windows and Linux users looking for a Finder they
 * do not have.
 */
export const REVEAL_LABEL = isMac
  ? "Reveal in Finder"
  : isWindows
    ? "Show in File Explorer"
    : "Show in File Manager";
