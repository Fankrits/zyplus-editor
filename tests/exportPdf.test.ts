import { describe, it, expect } from "bun:test";
import { renderPrintDocument } from "../src/lib/exportPdf";

describe("renderPrintDocument", () => {
  it("renders markdown to a standalone, print-styled document", () => {
    const html = renderPrintDocument("Notes", "# Title\n\nText with `code`.\n");
    expect(html.startsWith("<!doctype html>")).toBe(true);
    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<code>code</code>");
    // The page box is what makes the export paginate instead of clipping to one page.
    expect(html).toContain("@page");
    // Self-contained: nothing external to fetch. The one inline script is the
    // readiness ping the native exporter waits on, so it must stay.
    expect(html).not.toContain("<link");
    expect(html).not.toContain("src=");
    expect(html).toContain('fetch("/ready")');
  });

  it("renders GFM tables and task lists", () => {
    const html = renderPrintDocument("t", "| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n");
    expect(html).toContain("<table>");
    expect(html).toContain("<td>1</td>");
    expect(html).toContain('type="checkbox"');
  });

  it("escapes the title so a document name cannot inject markup", () => {
    const html = renderPrintDocument('</title><script>x</script>', "hi");
    expect(html).toContain("&lt;/title&gt;");
    expect(html).not.toContain("<script>x</script>");
  });
});
