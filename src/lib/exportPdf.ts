import { marked } from "marked";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { save as saveDialog, message } from "@tauri-apps/plugin-dialog";
import { isMac } from "./platform";
import { withExtension } from "./fs";

/**
 * "Export as PDF" builds a standalone HTML document and prints *that* straight
 * to a file the user names — the same save dialog "Export as .md" uses, no
 * print dialog in the way. Printing the app window instead meant hiding the
 * chrome with `@media print` while every ancestor stayed
 * `height:100%; overflow:hidden`, which clipped the output to a single page,
 * and plain-text mode only ever printed the lines CodeMirror had rendered into
 * the viewport.
 *
 * ponytail: `marked` renders GFM only — mermaid, KaTeX and GitHub alert
 * callouts come out as plain code blocks / blockquotes. Reuse the rich
 * editor's live DOM instead if those need to survive the export.
 */

const escapeHtml = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

// Print-only stylesheet: the app's themes are irrelevant on paper, so this is a
// plain black-on-white document sized for the page box rather than a viewport.
const DOCUMENT_CSS = `
  /* Honoured by the browser fallback; the native exporter sets its own
     printable area on NSPrintInfo instead (see start_print_to_pdf). */
  @page { margin: 18mm 16mm; }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    color: #000;
    background: #fff;
    font: 11pt/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    -webkit-print-color-adjust: exact;
    print-color-adjust: exact;
  }
  h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 1.4em 0 0.5em; break-after: avoid; }
  h1 { font-size: 1.9em; } h2 { font-size: 1.5em; } h3 { font-size: 1.25em; }
  h1:first-child, h2:first-child { margin-top: 0; }
  p, ul, ol, blockquote, table, pre { margin: 0 0 0.85em; }
  li { margin: 0.2em 0; }
  a { color: #000; text-decoration: underline; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 0.88em; }
  :not(pre) > code { background: #f2f2f2; padding: 0.1em 0.3em; border-radius: 3px; }
  pre {
    background: #f6f6f6; border: 1px solid #e0e0e0; border-radius: 4px;
    padding: 0.7em 0.9em; overflow-wrap: break-word; white-space: pre-wrap;
  }
  blockquote { border-left: 3px solid #ccc; margin-left: 0; padding-left: 1em; color: #333; }
  table { border-collapse: collapse; width: 100%; font-size: 0.95em; }
  th, td { border: 1px solid #bbb; padding: 0.4em 0.6em; text-align: left; }
  th { background: #f2f2f2; }
  img { max-width: 100%; }
  hr { border: 0; border-top: 1px solid #ccc; margin: 1.5em 0; }
  /* Keep atomic blocks off page seams. */
  pre, blockquote, table, img { break-inside: avoid; }
`;

/**
 * Wraps rendered markdown in a self-contained, print-ready HTML document.
 *
 * `signalReady` adds the script the native exporter waits on. The browser
 * fallback prints from the page that owns the document, needs no signal, and
 * gets a policy that allows no script at all.
 */
export function renderPrintDocument(title: string, markdown: string, signalReady = true): string {
  const body = marked.parse(markdown, { async: false, gfm: true, breaks: false });
  // `marked` passes raw HTML in a note straight through, and a note can come
  // from anyone. The policy lets the readiness ping below run and nothing else:
  // no <script> in the note, no onerror=, nothing fetched but images.
  const nonce = crypto.randomUUID();
  const csp =
    `default-src 'none'; script-src ${signalReady ? `'nonce-${nonce}'` : "'none'"}; style-src 'unsafe-inline'; ` +
    `img-src data: blob: https: http:; connect-src ${signalReady ? "'self'" : "'none'"}; base-uri 'none'; form-action 'none'`;
  const readyScript = signalReady
    ? `
<script nonce="${nonce}">
  // Tells the native exporter the document has laid out and can be printed.
  // An offscreen webview has no IPC, so the channel back is a request on the
  // same custom protocol that served this page.
  addEventListener("load", function () {
    fetch("/ready").catch(function () {});
  });
</script>`
    : "";
  return `<!doctype html>
<html><head><meta charset="utf-8" />
<meta http-equiv="Content-Security-Policy" content="${csp}" />
<title>${escapeHtml(title)}</title>
<style>${DOCUMENT_CSS}</style>
</head><body>${body}${readyScript}
</body></html>`;
}

/**
 * The native exporter is implemented against AppKit's `NSPrintOperation`
 * (see `start_print_to_pdf` in src-tauri/src/lib.rs), which has no Windows or
 * Linux counterpart — `export_pdf` refuses on those platforms. Checking here
 * too means a user who picks the action gets told that instead of being walked
 * through a save dialog whose file never appears.
 */
export const isPdfExportSupported = (): boolean => !isTauri() || isMac;

/**
 * Asks where to put the PDF, then writes it there. Resolves once the file
 * exists, or once the failure has been reported to the user — every caller is
 * fire-and-forget, so a rejection here would surface as nothing at all.
 */
export async function exportPdf(title: string, markdown: string): Promise<void> {
  // The native side has one print window and one ready signal; a second export
  // started before the first finished printed the wrong document or timed out.
  if (exporting) return;
  exporting = true;
  try {
    await runExport(title, markdown);
  } catch (err) {
    await reportFailure(err);
  } finally {
    exporting = false;
  }
}

let exporting = false;

async function runExport(title: string, markdown: string): Promise<void> {
  const defaultName = withExtension(title, ".pdf");

  if (!isTauri()) {
    await printInBrowser(title, renderPrintDocument(title, markdown, false));
    return;
  }

  const html = renderPrintDocument(title, markdown);
  if (!isPdfExportSupported()) {
    await message(
      "Exporting to PDF is only available on macOS right now. " +
        "Use \u201cExport as .md\u201d to save the document, or print it from another app.",
      { title: "Export as PDF", kind: "warning" },
    );
    return;
  }

  const path = await saveDialog({
    defaultPath: defaultName,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return; // cancelled
  await invoke("export_pdf", { html, path });
}

/**
 * Browser fallback: no native print job to run, so the document is printed
 * through the browser's own dialog ("Save as PDF" is one of its destinations).
 *
 * It prints from a hidden iframe rather than a popup. A popup opened after the
 * exporter's chunk had loaded asynchronously no longer counted as a response to
 * the click, so browsers blocked it, and the user saw nothing.
 */
async function printInBrowser(title: string, html: string): Promise<void> {
  const url = URL.createObjectURL(new Blob([html], { type: "text/html" }));
  // Not display:none — some browsers print a hidden frame as a blank page.
  const frame = Object.assign(document.createElement("iframe"), { src: url, title: "Print preview" });
  frame.setAttribute("aria-hidden", "true");
  frame.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;";
  // The browser suggests the top-level page's title as the PDF's file name.
  const pageTitle = document.title;

  await new Promise<void>((resolve, reject) => {
    let fallback: ReturnType<typeof setTimeout> | undefined;
    const finish = (err?: unknown) => {
      clearTimeout(fallback);
      document.title = pageTitle;
      frame.remove();
      URL.revokeObjectURL(url);
      if (err) reject(err);
      else resolve();
    };
    frame.onload = () => {
      try {
        const win = frame.contentWindow;
        if (!win) throw new Error("The print preview could not be opened.");
        document.title = withExtension(title, "");
        win.addEventListener("afterprint", () => finish(), { once: true });
        // Not every browser fires afterprint, and the frame must not leak.
        fallback = setTimeout(() => finish(), 60_000);
        win.focus();
        win.print();
      } catch (err) {
        finish(err);
      }
    };
    frame.onerror = () => finish(new Error("The print preview could not be loaded."));
    document.body.append(frame);
  });
}

async function reportFailure(err: unknown): Promise<void> {
  const detail = err instanceof Error ? err.message : String(err);
  console.error("Export as PDF failed:", err);
  if (!isTauri()) {
    window.alert(`Export as PDF\n\nThe PDF could not be created.\n\n${detail}`);
    return;
  }
  try {
    await message(`The PDF could not be written.\n\n${detail}`, {
      title: "Export as PDF",
      kind: "error",
    });
  } catch {
    // The dialog itself failing is not worth a second failure path.
  }
}
