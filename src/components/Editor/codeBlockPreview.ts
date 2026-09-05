import katex from "katex";
import { isDarkTheme } from "../../lib/theme";
import mermaid from "mermaid";
import { LanguageDescription } from "@codemirror/language";
import { languages as defaultLanguages } from "@codemirror/language-data";
import { markdown } from "@codemirror/lang-markdown";

// Ensure CSSRuleList is iterable in WebKit / Safari environments (e.g. macOS Tauri webview)
if (typeof CSSRuleList !== "undefined" && !CSSRuleList.prototype[Symbol.iterator]) {
  CSSRuleList.prototype[Symbol.iterator] = Array.prototype[Symbol.iterator];
}

// Fallback for headless or mock environments where SVG getBBox is not implemented
const svgProto = typeof SVGElement !== "undefined" ? (SVGElement.prototype as unknown as { getBBox?: () => { x: number; y: number; width: number; height: number } }) : null;
if (svgProto && !svgProto.getBBox) {
  svgProto.getBBox = function () {
    return { x: 0, y: 0, width: 100, height: 40 };
  };
}

export const mermaidLanguage = LanguageDescription.of({
  name: "Mermaid",
  alias: ["mermaid", "flowchart", "diagram"],
  load: async () => markdown(),
});

export const supportedLanguages: LanguageDescription[] = [
  mermaidLanguage,
  ...defaultLanguages,
];

const DIAGRAM_LANGUAGES = new Set(["mermaid", "flowchart", "diagram"]);
const MATH_LANGUAGES = new Set(["latex", "math", "katex", "tex"]);

export function getMermaidConfig() {
  // Mermaid bakes these into the SVG, so they can't be CSS variables; they mirror
  // the --diagram-* tokens in App.css. Flat background fills, distinct strokes.
  const dark = isDarkTheme();
  const fill = dark ? "#18181b" : "#ffffff";

  const primaryFill = fill;
  const primaryStroke = dark ? "#63b3ff" : "#006fee";
  const primaryText = primaryStroke;

  const secondaryFill = fill;
  const secondaryStroke = dark ? "#c084fc" : "#7828c8";
  const secondaryText = secondaryStroke;

  const tertiaryFill = fill;
  const tertiaryStroke = dark ? "#34d399" : "#17c964";
  const tertiaryText = dark ? "#34d399" : "#16a34a";

  const warningFill = fill;
  const warningStroke = dark ? "#fbbf24" : "#f5a524";
  const warningText = dark ? "#fbbf24" : "#d97706";

  const lineStroke = dark ? "#a1a1aa" : "#64748b";
  const clusterFill = fill;
  const clusterBorder = dark ? "#3f3f46" : "#cbd5e1";
  const edgeLabelBg = fill;
  const textColor = dark ? "#e4e4e7" : "#0f172a";

  return {
    startOnLoad: false,
    securityLevel: "loose" as const,
    suppressErrorRendering: true,
    theme: "base" as const,
    flowchart: {
      htmlLabels: true,
      padding: 20,
    },
    themeCSS: `
      foreignObject, foreignObject > div, .label foreignObject, .cluster-label foreignObject {
        overflow: visible !important;
      }
      .node .label, .cluster-label {
        overflow: visible !important;
      }
      foreignObject p, .nodeLabel p {
        margin: 0 !important;
        padding: 0 0 2px 0 !important;
        line-height: 1.35 !important;
      }
      .cluster rect {
        stroke-dasharray: none !important;
        rx: 0 !important;
        ry: 0 !important;
      }
      .node rect {
        rx: 0 !important;
        ry: 0 !important;
      }
    `,
    themeVariables: {
      darkMode: dark,
      background: fill,
      fontFamily:
        "ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
      fontSize: "14px",

      // Flowchart primary nodes
      primaryColor: primaryFill,
      primaryBorderColor: primaryStroke,
      primaryTextColor: primaryText,

      // Secondary
      secondaryColor: secondaryFill,
      secondaryBorderColor: secondaryStroke,
      secondaryTextColor: secondaryText,

      // Tertiary
      tertiaryColor: tertiaryFill,
      tertiaryBorderColor: tertiaryStroke,
      tertiaryTextColor: tertiaryText,

      // Main / Base defaults
      mainBkg: primaryFill,
      nodeBorder: primaryStroke,
      nodeTextColor: primaryText,

      // Connectors, links & arrows
      lineColor: lineStroke,
      arrowheadColor: lineStroke,
      defaultLinkColor: lineStroke,

      // Subgraphs / Clusters
      clusterBkg: clusterFill,
      clusterBorder: clusterBorder,
      titleColor: textColor,

      // Edge labels
      edgeLabelBackground: edgeLabelBg,
      textColor: textColor,

      // Notes
      noteBkgColor: warningFill,
      noteBorderColor: warningStroke,
      noteTextColor: warningText,

      // State diagrams
      stateBkg: primaryFill,
      stateLabelColor: primaryText,
      transitionColor: lineStroke,
      transitionLabelColor: textColor,

      // Sequence diagrams
      actorBkg: primaryFill,
      actorBorder: primaryStroke,
      actorTextColor: primaryText,
      actorLineColor: clusterBorder,
      signalColor: lineStroke,
      signalTextColor: textColor,
      labelBoxBkgColor: secondaryFill,
      labelBoxBorderColor: secondaryStroke,
      labelTextColor: secondaryText,
      activationBkgColor: "rgba(0, 111, 238, 0.15)",
      activationBorderColor: primaryStroke,

      // Class diagrams
      classText: primaryText,

      // Git Graph
      git0: "#006fee",
      git1: "#7828c8",
      git2: "#17c964",
      git3: "#f5a524",
      git4: "#f31260",
      git5: "#06b6d4",
      git6: "#ec4899",
      git7: "#6366f1",
      gitBranchLabel0: "#004493",
      gitBranchLabel1: "#481878",
      gitBranchLabel2: "#0e793c",
      gitBranchLabel3: "#92400e",
      gitBranchLabel4: "#991b1b",
      gitBranchLabel5: "#155e75",
      gitBranchLabel6: "#9d174d",
      gitBranchLabel7: "#3730a3",

      // Pie Chart
      pie1: "#006fee",
      pie2: "#7828c8",
      pie3: "#17c964",
      pie4: "#f5a524",
      pie5: "#f31260",
      pie6: "#06b6d4",
      pie7: "#ec4899",
      pieTitleTextColor: textColor,
      pieSectionTextColor: fill,
      pieLegendTextColor: textColor,
      pieStrokeColor: fill,
      pieStrokeWidth: "2px",
    },
  };
}

function injectMulticolorStyles(svg: string, renderId: string): string {
  const bg = "var(--diagram-bg)";
  const styles = `
    #${renderId} foreignObject,
    #${renderId} foreignObject > div,
    #${renderId} .label foreignObject,
    #${renderId} .cluster-label foreignObject {
      overflow: visible !important;
    }
    #${renderId} .node .label,
    #${renderId} .cluster-label {
      overflow: visible !important;
    }
    #${renderId} foreignObject p,
    #${renderId} .nodeLabel p {
      margin: 0 !important;
      padding: 0 0 2px 0 !important;
      line-height: 1.35 !important;
    }
    #${renderId} foreignObject span,
    #${renderId} .nodeLabel {
      display: inline-block !important;
      line-height: 1.35 !important;
      overflow: visible !important;
    }
    #${renderId} .cluster-label :is(span, text, p),
    #${renderId} .cluster .nodeLabel {
      color: var(--diagram-text) !important;
      fill: var(--diagram-text) !important;
      font-weight: 600 !important;
      font-size: 13px !important;
    }

    #${renderId} .node :is(rect, polygon, circle, ellipse, path.basic, .outer-path path) { stroke-width: 1.75px !important; }
    #${renderId} .node:not(.cluster) rect { rx: 0 !important; ry: 0 !important; }
    #${renderId} .cluster rect { rx: 0 !important; ry: 0 !important; stroke-width: 1.5px !important; stroke-dasharray: none !important; fill: ${bg} !important; stroke: var(--diagram-border) !important; }

    /* 1. Vibrant Blue */
    #${renderId} g > .node:nth-of-type(6n + 1) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-1) !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-1) !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-1-text) !important; fill: var(--diagram-1-text) !important; font-weight: 600 !important; }

    /* 2. Vibrant Purple */
    #${renderId} g > .node:nth-of-type(6n + 2) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-2) !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-2) !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-2-text) !important; fill: var(--diagram-2-text) !important; font-weight: 600 !important; }

    /* 3. Vibrant Emerald */
    #${renderId} g > .node:nth-of-type(6n + 3) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-3) !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-3) !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-3-text) !important; fill: var(--diagram-3-text) !important; font-weight: 600 !important; }

    /* 4. Vibrant Amber */
    #${renderId} g > .node:nth-of-type(6n + 4) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-4) !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-4) !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-4-text) !important; fill: var(--diagram-4-text) !important; font-weight: 600 !important; }

    /* 5. Vibrant Rose */
    #${renderId} g > .node:nth-of-type(6n + 5) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-5) !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-5) !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-5-text) !important; fill: var(--diagram-5-text) !important; font-weight: 600 !important; }

    /* 6. Vibrant Cyan */
    #${renderId} g > .node:nth-of-type(6n + 6) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-6) !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${bg} !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: var(--diagram-6) !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-6-text) !important; fill: var(--diagram-6-text) !important; font-weight: 600 !important; }
  `;

  if (svg.includes("</svg>")) {
    return svg.replace("</svg>", `<style>${styles}</style></svg>`);
  }
  return svg;
}

export function renderCodeBlockPreview(
  language: string,
  content: string,
  applyPreview: (value: null | string | HTMLElement) => void,
): void | null | string | HTMLElement {
  const trimmed = content.trim();
  if (!trimmed) return null;

  const lang = language.trim().toLowerCase();

  if (MATH_LANGUAGES.has(lang)) {
    try {
      return katex.renderToString(trimmed, {
        throwOnError: false,
        displayMode: true,
      });
    } catch {
      return null;
    }
  }

  if (DIAGRAM_LANGUAGES.has(lang)) {
    mermaid.initialize(getMermaidConfig());

    const renderId = `mermaid-${Math.random().toString(36).slice(2, 9)}`;
    mermaid
      .render(renderId, trimmed)
      .then(({ svg }) => {
        applyPreview(injectMulticolorStyles(svg, renderId));
      })
      .catch(() => {
        applyPreview(null);
      });

    return undefined;
  }

  return null;
}
