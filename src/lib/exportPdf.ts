import { marked } from "marked";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";

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

/** Wraps rendered markdown in a self-contained, print-ready HTML document. */
export function renderPrintDocument(title: string, markdown: string): string {
  const body = marked.parse(markdown, { async: false, gfm: true, breaks: false });
  return `<!doctype html>
<html><head><meta charset="utf-8" />
<title>${escapeHtml(title)}</title>
<style>${DOCUMENT_CSS}</style>
</head><body>${body}
<script>
  // Tells the native exporter the document has laid out and can be printed.
  // An offscreen webview has no IPC, so the channel back is a request on the
  // same custom protocol that served this page.
  addEventListener("load", function () {
    fetch("/ready");
  });
</script>
</body></html>`;
}

/**
 * Asks where to put the PDF, then writes it there. Resolves once the file
 * exists; rejects with the reason if it could not be written.
 */
export async function exportPdf(title: string, markdown: string): Promise<void> {
  const html = renderPrintDocument(title, markdown);
  const defaultName = title.replace(/\.[^.]+$/, "") + ".pdf";

  if (!isTauri()) {
    // Browser dev: no native print job to run, so fall back to the print dialog.
    const w = window.open("", "_blank");
    if (!w) return;
    w.document.write(html);
    w.document.close();
    w.addEventListener("load", () => w.print());
    return;
  }

  const path = await saveDialog({
    defaultPath: defaultName,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (!path) return; // cancelled
  await invoke("export_pdf", { html, path });
}
