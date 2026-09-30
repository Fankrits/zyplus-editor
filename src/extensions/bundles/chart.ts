import { Chart, registerables } from "chart.js";
import type { ExtensionRuntime } from "../types";

Chart.register(...registerables);

/**
 * Expected schema:
 * {
 *   "type": "bar" | "line" | "pie" | "doughnut" | "scatter",
 *   "data": {
 *     "labels": [...],
 *     "datasets": [{ "label": string, "data": [...], ... }]
 *   },
 *   "options": { ... }
 * }
 */

function getThemeColors() {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  return {
    text: style.getPropertyValue("--diagram-text").trim() || "#0f172a",
    border: style.getPropertyValue("--diagram-border").trim() || "#cbd5e1",
    palette: [
      style.getPropertyValue("--diagram-1").trim() || "#006fee",
      style.getPropertyValue("--diagram-2").trim() || "#7828c8",
      style.getPropertyValue("--diagram-3").trim() || "#17c964",
      style.getPropertyValue("--diagram-4").trim() || "#f5a524",
      style.getPropertyValue("--diagram-5").trim() || "#f31260",
      style.getPropertyValue("--diagram-6").trim() || "#06b6d4",
    ],
  };
}

function applyThemeToConfig(rawConfig: any) {
  // Deep clone to avoid mutating the original
  const cfg = JSON.parse(JSON.stringify(rawConfig));
  const theme = getThemeColors();

  if (!cfg.options) cfg.options = {};
  if (!cfg.options.plugins) cfg.options.plugins = {};
  if (!cfg.options.plugins.legend) cfg.options.plugins.legend = {};
  if (!cfg.options.plugins.legend.labels) cfg.options.plugins.legend.labels = {};

  // The canvas here is rendered fully offscreen and rasterized once (see
  // renderCodeBlockPreview) rather than kept live in the document, so its size has to
  // be a fixed value Chart.js is told about up front, not something it measures from a
  // parent element. `animation: false` makes that render happen synchronously inside
  // `new Chart(...)`, so `toDataURL()` can be called immediately after, with no
  // async/rAF wait for a paint that a detached canvas would never get anyway.
  cfg.options.responsive = false;
  cfg.options.maintainAspectRatio = false;
  cfg.options.animation = false;

  // Set default text color
  if (cfg.options.color === undefined) cfg.options.color = theme.text;

  // Apply palette to datasets
  if (cfg.data && Array.isArray(cfg.data.datasets)) {
    cfg.data.datasets.forEach((ds: any, i: number) => {
      const color = theme.palette[i % theme.palette.length];
      if (!ds.backgroundColor) ds.backgroundColor = color;
      if (!ds.borderColor) ds.borderColor = color;
    });
  }

  // Set grid lines colors for cartesian charts
  if (["line", "bar", "scatter"].includes(cfg.type)) {
    if (!cfg.options.scales) cfg.options.scales = {};
    const scales = cfg.options.scales;

    // Set for each scale explicitly provided, or provide defaults if empty
    if (Object.keys(scales).length === 0) {
      scales.x = { grid: { color: theme.border }, ticks: { color: theme.text } };
      scales.y = { grid: { color: theme.border }, ticks: { color: theme.text } };
    } else {
      for (const key of Object.keys(scales)) {
        const scale = scales[key];
        if (!scale || typeof scale !== "object") continue;
        if (!scale.grid) scale.grid = {};
        if (!scale.grid.color) scale.grid.color = theme.border;
        if (!scale.ticks) scale.ticks = {};
        if (!scale.ticks.color) scale.ticks.color = theme.text;
      }
    }
  }

  return cfg;
}

export default function createChartExtension(): ExtensionRuntime {
  return {
    id: "chart",
    renderCodeBlockPreview(
      _language: string,
      content: string,
      applyPreview: (value: null | string | HTMLElement) => void,
    ) {
      const trimmed = content.trim();
      if (!trimmed) return null;

      let config;
      try {
        config = JSON.parse(trimmed);
      } catch (e) {
        return null;
      }

      if (!config || !config.type || !config.data || !Array.isArray(config.data.datasets)) {
        return null;
      }

      try {
        // Rendered fully offscreen (never appended anywhere) and rasterized once to a
        // static PNG, the same way mermaid.ts hands back a plain SVG and katex.ts a
        // plain HTML string, rather than a live, still-updatable element. A live
        // Chart.js canvas came back empty in the real editor every time: whatever
        // Crepe's code-block-preview host does to place the returned node, a `<canvas>`
        // does not survive it — a canvas's drawn bitmap has no textual/DOM
        // representation at all, unlike an SVG's markup or an HTML string, and the
        // symptom matched exactly (the canvas kept its width/height attributes, which
        // do have one, and lost only the pixels, which don't). Rasterizing sidesteps
        // the whole question of what that host mechanism is, at the cost of an
        // interaction-free image — an acceptable trade in a read-only note preview,
        // and the same one mermaid/katex already make.
        const canvas = document.createElement("canvas");
        canvas.width = 760;
        canvas.height = 428;
        const themedConfig = applyThemeToConfig(config);
        const chart = new Chart(canvas, themedConfig);
        let dataUrl: string;
        try {
          dataUrl = canvas.toDataURL("image/png");
        } finally {
          chart.destroy();
        }

        const container = document.createElement("div");
        // Match Mermaid/KaTeX block layout: centered, capped width.
        container.style.width = "100%";
        container.style.maxWidth = "800px";
        container.style.margin = "0 auto 1em";

        const img = document.createElement("img");
        img.src = dataUrl;
        img.alt = "Chart preview";
        img.style.display = "block";
        img.style.width = "100%";
        img.style.height = "auto";
        container.appendChild(img);

        applyPreview(container);
      } catch (e) {
        return null;
      }

      return undefined; // We called applyPreview synchronously
    },
  };
}
