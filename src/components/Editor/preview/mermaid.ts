import mermaid from "mermaid";
import { isDarkTheme } from "../../../lib/theme";

/**
 * Only the parts of the diagram that milkdown-heroui-theme.css does not reach:
 * sequence actors, notes, signals. Flowchart nodes, edges, labels and clusters
 * are painted there instead, off the --diagram-* tokens, so they follow a theme
 * change without re-rendering.
 */
function config(isDark: boolean) {
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

let initializedDark: boolean | null = null;

/** Renders a ```mermaid block to SVG. */
export async function render(source: string): Promise<string | null> {
  const isDark = isDarkTheme();
  if (initializedDark !== isDark) {
    mermaid.initialize(config(isDark));
    initializedDark = isDark;
  }
  try {
    const id = `mermaid-${Math.random().toString(36).slice(2, 9)}`;
    return (await mermaid.render(id, source)).svg;
  } catch {
    return null;
  }
}
