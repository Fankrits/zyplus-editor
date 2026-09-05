import katex from "katex";
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

export function getMermaidConfig(isDark: boolean) {
  // Brighter luminous background fills and distinct theme strokes
  const primaryFill = isDark ? "#0c2b5e" : "#f0f7ff";
  const primaryStroke = isDark ? "#38bdf8" : "#006fee";
  const primaryText = isDark ? "#e0f2fe" : "#1e40af";

  const secondaryFill = isDark ? "#2e124d" : "#faf5ff";
  const secondaryStroke = isDark ? "#c084fc" : "#7828c8";
  const secondaryText = isDark ? "#f3e8ff" : "#581c87";

  const tertiaryFill = isDark ? "#062e1a" : "#f0fdf4";
  const tertiaryStroke = isDark ? "#4ade80" : "#17c964";
  const tertiaryText = isDark ? "#dcfce7" : "#14532d";

  const warningFill = isDark ? "#2d2006" : "#fffbeb";
  const warningStroke = isDark ? "#fbbf24" : "#f5a524";
  const warningText = isDark ? "#fef3c7" : "#78350f";

  const lineStroke = isDark ? "#94a3b8" : "#475569";
  const clusterFill = isDark ? "rgba(39, 39, 42, 0.4)" : "#fafafa";
  const clusterBorder = isDark ? "#52525b" : "#d4d4d8";
  const edgeLabelBg = isDark ? "#18181b" : "#ffffff";
  const textColor = isDark ? "#f4f4f5" : "#0f172a";

  return {
    startOnLoad: false,
    securityLevel: "loose" as const,
    suppressErrorRendering: true,
    theme: "base" as const,
    themeVariables: {
      darkMode: isDark,
      background: "transparent",
      fontFamily:
        "ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
      fontSize: "14px",

      // Flowchart primary nodes (HeroUI Blue)
      primaryColor: primaryFill,
      primaryBorderColor: primaryStroke,
      primaryTextColor: primaryText,

      // Secondary (HeroUI Purple)
      secondaryColor: secondaryFill,
      secondaryBorderColor: secondaryStroke,
      secondaryTextColor: secondaryText,

      // Tertiary (HeroUI Success Emerald)
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

      // Notes (HeroUI Amber)
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
      activationBkgColor: isDark ? "rgba(0, 111, 238, 0.25)" : "rgba(0, 111, 238, 0.15)",
      activationBorderColor: primaryStroke,

      // Class diagrams
      classText: primaryText,

      // Git Graph (HeroUI semantic palette)
      git0: "#006fee",
      git1: "#7828c8",
      git2: "#17c964",
      git3: "#f5a524",
      git4: "#f31260",
      git5: "#06b6d4",
      git6: "#ec4899",
      git7: "#6366f1",
      gitBranchLabel0: isDark ? "#93c5fd" : "#004493",
      gitBranchLabel1: isDark ? "#c4b5fd" : "#481878",
      gitBranchLabel2: isDark ? "#6ee7b7" : "#0e793c",
      gitBranchLabel3: isDark ? "#fde047" : "#92400e",
      gitBranchLabel4: isDark ? "#fca5a5" : "#991b1b",
      gitBranchLabel5: isDark ? "#67e8f9" : "#155e75",
      gitBranchLabel6: isDark ? "#f472b6" : "#9d174d",
      gitBranchLabel7: isDark ? "#a5b4fc" : "#3730a3",

      // Pie Chart
      pie1: "#006fee",
      pie2: "#7828c8",
      pie3: "#17c964",
      pie4: "#f5a524",
      pie5: "#f31260",
      pie6: "#06b6d4",
      pie7: "#ec4899",
      pieTitleTextColor: textColor,
      pieSectionTextColor: "#ffffff",
      pieLegendTextColor: isDark ? "#e4e4e7" : "#3f3f46",
      pieStrokeColor: isDark ? "#18181b" : "#ffffff",
      pieStrokeWidth: "2px",
    },
  };
}

function injectMulticolorStyles(svg: string, renderId: string, isDark: boolean): string {
  const styles = `
    #${renderId} .node :is(rect, polygon, circle, ellipse, path.basic, .outer-path path) { stroke-width: 1.75px !important; }
    #${renderId} .node:not(.cluster) rect { rx: 0 !important; ry: 0 !important; }
    #${renderId} .cluster rect { rx: 0 !important; ry: 0 !important; stroke-width: 1.5px !important; stroke-dasharray: 4 4 !important; }

    /* 1. Vibrant Blue */
    #${renderId} g > .node:nth-of-type(6n + 1) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#0c2b5e" : "#eff6ff"} !important; stroke: ${isDark ? "#38bdf8" : "#006fee"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#0c2b5e" : "#eff6ff"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#38bdf8" : "#006fee"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#e0f2fe" : "#1e40af"} !important; fill: ${isDark ? "#e0f2fe" : "#1e40af"} !important; font-weight: 600 !important; }

    /* 2. Vibrant Purple */
    #${renderId} g > .node:nth-of-type(6n + 2) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#2e124d" : "#faf5ff"} !important; stroke: ${isDark ? "#c084fc" : "#7828c8"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#2e124d" : "#faf5ff"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#c084fc" : "#7828c8"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#f3e8ff" : "#581c87"} !important; fill: ${isDark ? "#f3e8ff" : "#581c87"} !important; font-weight: 600 !important; }

    /* 3. Vibrant Emerald */
    #${renderId} g > .node:nth-of-type(6n + 3) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#062e1a" : "#f0fdf4"} !important; stroke: ${isDark ? "#4ade80" : "#17c964"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#062e1a" : "#f0fdf4"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#4ade80" : "#17c964"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#dcfce7" : "#14532d"} !important; fill: ${isDark ? "#dcfce7" : "#14532d"} !important; font-weight: 600 !important; }

    /* 4. Vibrant Amber */
    #${renderId} g > .node:nth-of-type(6n + 4) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#2d2006" : "#fffbeb"} !important; stroke: ${isDark ? "#fbbf24" : "#f5a524"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#2d2006" : "#fffbeb"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#fbbf24" : "#f5a524"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 4) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#fef3c7" : "#78350f"} !important; fill: ${isDark ? "#fef3c7" : "#78350f"} !important; font-weight: 600 !important; }

    /* 5. Vibrant Rose */
    #${renderId} g > .node:nth-of-type(6n + 5) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#360c1c" : "#fff1f2"} !important; stroke: ${isDark ? "#fb7185" : "#f31260"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#360c1c" : "#fff1f2"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#fb7185" : "#f31260"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 5) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#ffe4e6" : "#881337"} !important; fill: ${isDark ? "#ffe4e6" : "#881337"} !important; font-weight: 600 !important; }

    /* 6. Vibrant Cyan */
    #${renderId} g > .node:nth-of-type(6n + 6) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${isDark ? "#082f38" : "#ecfeff"} !important; stroke: ${isDark ? "#22d3ee" : "#06b6d4"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) .outer-path path[stroke="none"]:not([style*="fill"]) { fill: ${isDark ? "#082f38" : "#ecfeff"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) .outer-path path:not([stroke="none"]):not([style*="stroke"]) { stroke: ${isDark ? "#22d3ee" : "#06b6d4"} !important; }
    #${renderId} g > .node:nth-of-type(6n + 6) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: ${isDark ? "#cffafe" : "#155e75"} !important; fill: ${isDark ? "#cffafe" : "#155e75"} !important; font-weight: 600 !important; }
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
    const isDark =
      typeof document !== "undefined" &&
      (document.documentElement.classList.contains("dark") ||
        (typeof window !== "undefined" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches));

    mermaid.initialize(getMermaidConfig(isDark));

    const renderId = `mermaid-${Math.random().toString(36).slice(2, 9)}`;
    mermaid
      .render(renderId, trimmed)
      .then(({ svg }) => {
        applyPreview(injectMulticolorStyles(svg, renderId, isDark));
      })
      .catch(() => {
        applyPreview(null);
      });

    return undefined;
  }

  return null;
}
