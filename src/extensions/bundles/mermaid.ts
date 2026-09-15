import mermaid from "mermaid";
import type { ExtensionContext, ExtensionRuntime } from "../types";

function getMermaidConfig(isDark: boolean) {
  const fill = isDark ? "#18181b" : "#ffffff";
  const primaryStroke = isDark ? "#63b3ff" : "#006fee";
  const secondaryStroke = isDark ? "#c084fc" : "#7828c8";
  const tertiaryStroke = isDark ? "#34d399" : "#17c964";
  const warningStroke = isDark ? "#fbbf24" : "#f5a524";
  const lineStroke = isDark ? "#a1a1aa" : "#64748b";
  const clusterBorder = isDark ? "#3f3f46" : "#cbd5e1";
  const textColor = isDark ? "#e4e4e7" : "#0f172a";

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
      darkMode: isDark,
      background: fill,
      fontFamily:
        "ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif",
      fontSize: "14px",
      primaryColor: fill,
      primaryBorderColor: primaryStroke,
      primaryTextColor: primaryStroke,
      secondaryColor: fill,
      secondaryBorderColor: secondaryStroke,
      secondaryTextColor: secondaryStroke,
      tertiaryColor: fill,
      tertiaryBorderColor: tertiaryStroke,
      tertiaryTextColor: isDark ? "#34d399" : "#16a34a",
      mainBkg: fill,
      nodeBorder: primaryStroke,
      nodeTextColor: primaryStroke,
      lineColor: lineStroke,
      arrowheadColor: lineStroke,
      defaultLinkColor: lineStroke,
      clusterBkg: fill,
      clusterBorder: clusterBorder,
      titleColor: textColor,
      edgeLabelBackground: fill,
      textColor: textColor,
      noteBkgColor: fill,
      noteBorderColor: warningStroke,
      noteTextColor: isDark ? "#fbbf24" : "#d97706",
      actorBkg: fill,
      actorBorder: primaryStroke,
      actorTextColor: primaryStroke,
      actorLineColor: clusterBorder,
      signalColor: lineStroke,
      signalTextColor: textColor,
      labelBoxBkgColor: fill,
      labelBoxBorderColor: secondaryStroke,
      labelTextColor: secondaryStroke,
      activationBkgColor: "rgba(0, 111, 238, 0.15)",
      activationBorderColor: primaryStroke,
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
    #${renderId} g > .node:nth-of-type(6n + 1) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-1) !important; }
    #${renderId} g > .node:nth-of-type(6n + 1) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-1-text) !important; fill: var(--diagram-1-text) !important; font-weight: 600 !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-2) !important; }
    #${renderId} g > .node:nth-of-type(6n + 2) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-2-text) !important; fill: var(--diagram-2-text) !important; font-weight: 600 !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) :is(rect, polygon, circle, ellipse, path.basic):not([style*="fill"]) { fill: ${bg} !important; stroke: var(--diagram-3) !important; }
    #${renderId} g > .node:nth-of-type(6n + 3) :is(.label span, .label text, text):not([style*="color"]):not([style*="fill"]) { color: var(--diagram-3-text) !important; fill: var(--diagram-3-text) !important; font-weight: 600 !important; }
  `;

  if (svg.includes("</svg>")) {
    return svg.replace("</svg>", `<style>${styles}</style></svg>`);
  }
  return svg;
}

export default function createMermaidExtension(): ExtensionRuntime {
  let context: ExtensionContext | null = null;
  let lastThemeDark: boolean | null = null;

  return {
    id: "mermaid",
    activate(ctx: ExtensionContext) {
      context = ctx;
      const isDark = ctx.isDarkTheme();
      mermaid.initialize(getMermaidConfig(isDark));
      lastThemeDark = isDark;
    },
    renderCodeBlockPreview(
      _language: string,
      content: string,
      applyPreview: (value: null | string | HTMLElement) => void,
    ) {
      const trimmed = content.trim();
      if (!trimmed) return null;

      const isDark = context ? context.isDarkTheme() : false;
      if (lastThemeDark !== isDark) {
        mermaid.initialize(getMermaidConfig(isDark));
        lastThemeDark = isDark;
      }

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
    },
    deactivate() {
      context = null;
    },
  };
}
