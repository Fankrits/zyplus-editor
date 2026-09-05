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
  // HeroUI semantic palette tokens
  const primaryFill = isDark ? "#0c2b5e" : "#e6f1fe";
  const primaryStroke = isDark ? "#338ef7" : "#006fee";
  const primaryText = isDark ? "#bae6fd" : "#004493";

  const secondaryFill = isDark ? "#281245" : "#f2eafe";
  const secondaryStroke = isDark ? "#9353d3" : "#7828c8";
  const secondaryText = isDark ? "#e9d5ff" : "#481878";

  const tertiaryFill = isDark ? "#062e1a" : "#e8faf0";
  const tertiaryStroke = isDark ? "#17c964" : "#12a150";
  const tertiaryText = isDark ? "#a7f3d0" : "#0e793c";

  const warningFill = isDark ? "#2d2006" : "#fefce8";
  const warningStroke = isDark ? "#f5a524" : "#d97706";
  const warningText = isDark ? "#fde047" : "#92400e";

  const lineStroke = isDark ? "#338ef7" : "#006fee";
  const clusterFill = isDark ? "rgba(39, 39, 42, 0.45)" : "rgba(244, 244, 245, 0.75)";
  const clusterBorder = isDark ? "#3f3f46" : "#cbd5e1";
  const edgeLabelBg = isDark ? "#18181b" : "#ffffff";
  const textColor = isDark ? "#f4f4f5" : "#18181b";

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
        applyPreview(svg);
      })
      .catch(() => {
        applyPreview(null);
      });

    return undefined;
  }

  return null;
}
