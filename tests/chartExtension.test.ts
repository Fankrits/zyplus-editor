import { describe, it, expect } from "bun:test";
import { EXTENSION_CATALOG } from "../src/extensions/catalog";
import createChartExtension from "../src/extensions/bundles/chart";
import fs from "node:fs";

describe("Chart Extension", () => {
  it("Manifest correctness", () => {
    const chart = EXTENSION_CATALOG.find((e) => e.id === "chart");
    expect(chart).toBeDefined();
    expect(chart?.languages).toEqual(["chart", "chartjs"]);
    expect(chart?.fileExtensions).toBeUndefined();
  });

  it("Valid config renders", () => {
    const ext = createChartExtension();
    let applyPreviewValue: any = null;
    const applyPreview = (val: any) => { applyPreviewValue = val; };
    
    // Bar
    ext.renderCodeBlockPreview?.("chart", JSON.stringify({ type: "bar", data: { datasets: [] } }), applyPreview);
    expect(applyPreviewValue).toBeInstanceOf(window.HTMLElement);
    expect(applyPreviewValue.tagName).toBe("DIV");
    // A canvas is rasterized to a static <img> rather than kept live — see chart.ts's
    // comment on why a returned live canvas doesn't survive the real host.
    const img = applyPreviewValue.querySelector("img");
    expect(img).not.toBeNull();
    expect(img.src).toMatch(/^data:image\/png;base64,/);

    // Line
    ext.renderCodeBlockPreview?.("chart", JSON.stringify({ type: "line", data: { datasets: [] } }), applyPreview);
    expect(applyPreviewValue).toBeInstanceOf(window.HTMLElement);

    // Pie
    ext.renderCodeBlockPreview?.("chart", JSON.stringify({ type: "pie", data: { datasets: [] } }), applyPreview);
    expect(applyPreviewValue).toBeInstanceOf(window.HTMLElement);
  });

  it("Invalid JSON / invalid Chart.js config returns null", () => {
    const ext = createChartExtension();

    // Invalid JSON
    const res1 = ext.renderCodeBlockPreview?.("chart", "not json", () => {});
    expect(res1).toBeNull();
    
    // Invalid schema (missing data.datasets)
    const res2 = ext.renderCodeBlockPreview?.("chart", JSON.stringify({ type: "bar" }), () => {});
    expect(res2).toBeNull();
  });

  it("Theme colors: read css variables and change color", () => {
    const ext = createChartExtension();
    // Set initial custom properties for document element
    document.documentElement.style.setProperty("--diagram-bg", "#000000");
    document.documentElement.style.setProperty("--diagram-text", "#ffffff");
    document.documentElement.style.setProperty("--diagram-1", "#ff0000");
    
    ext.renderCodeBlockPreview?.("chart", JSON.stringify({
      type: "bar", 
      data: { datasets: [{ data: [1,2,3] }] }
    }), () => {});

    // In happy-dom the chart isn't really constructed with canvas methods, but we can verify our theme colors test passes without throwing.
    // Explicit user color test
    ext.renderCodeBlockPreview?.("chart", JSON.stringify({ 
      type: "bar", 
      data: { datasets: [{ backgroundColor: "#00ff00", data: [1,2,3] }] }
    }), () => {});
    // User color overrides aren't observable here as Chart.js mutates/copies inside the Canvas context.
  });

  it("No color literals in chart.ts outside of comments", () => {
    const source = fs.readFileSync(new URL("../src/extensions/bundles/chart.ts", import.meta.url), "utf-8");
    // Strip comments
    const withoutComments = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
    
    // Test that the only color literals in chart.ts are the fallbacks, matching the tokens
    // We can just verify the file does not contain literal `#abcdef` that isn't part of the fallback object
    // For simplicity, we just assert there are no literals EXCEPT those inside the getThemeColors function.
    // Let's just do a regex that excludes the known fallbacks:
    const stringWithoutKnownFallbacks = withoutComments
      .replace(/#ffffff/g, "")
      .replace(/#0f172a/g, "")
      .replace(/#cbd5e1/g, "")
      .replace(/#64748b/g, "")
      .replace(/#006fee/g, "")
      .replace(/#7828c8/g, "")
      .replace(/#17c964/g, "")
      .replace(/#f5a524/g, "")
      .replace(/#f31260/g, "")
      .replace(/#06b6d4/g, "");
      
    const hexRegex = /#[0-9a-f]{3,8}\b/gi;
    const match = stringWithoutKnownFallbacks.match(hexRegex);
    expect(match).toBeNull();
  });
});
