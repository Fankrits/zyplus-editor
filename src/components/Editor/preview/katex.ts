import katex from "katex";
// Bundled with this chunk, fonts and all — the stylesheet used to be fetched
// from a CDN, which left math unstyled on a machine that is offline.
import "katex/dist/katex.min.css";

/** Renders a ```latex block to HTML. */
export function render(source: string): string | null {
  try {
    return katex.renderToString(source, { throwOnError: false, displayMode: true });
  } catch {
    return null;
  }
}
