import { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";

export const plainTextTheme: Extension = EditorView.theme({
  "&": {
    height: "100%",
    background: "transparent",
  },
  "&.cm-focused": {
    outline: "none !important",
  },
  ".cm-scroller": {
    overflow: "auto",
    fontFamily: 'ui-monospace, "SF Mono", Menlo, Monaco, Consolas, "JetBrains Mono", monospace',
    lineHeight: "1.65",
    letterSpacing: "-0.01em",
  },
  ".cm-content": {
    maxWidth: "48rem",
    margin: "0 auto",
    padding: "1.5rem 1rem 8rem 1rem",
    fontSize: "15px",
    caretColor: "var(--accent, currentColor)",
  },
  "@media (min-width: 640px)": {
    ".cm-content": {
      padding: "3rem 1.5rem 12rem 1.5rem",
    },
  },
  ".cm-line": {
    padding: "0",
    maxWidth: "100%",
  },
  ".cm-activeLine": {
    backgroundColor: "transparent",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--accent-soft, rgba(0, 111, 238, 0.15)) !important",
  },
  ".cm-searchMatch": {
    backgroundColor: "var(--warning-soft, rgba(245, 165, 36, 0.25))",
    borderRadius: "3px",
  },
  ".cm-searchMatch-selected, .cm-searchMatch.cm-searchMatch-selected": {
    backgroundColor: "var(--warning, #f5a524)",
    color: "var(--background, #fff)",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--accent, #006fee)",
    borderLeftWidth: "2px",
  },
});

export const plainTextHighlighting: Extension = syntaxHighlighting(
  HighlightStyle.define([
    {
      tag: tags.heading,
      fontWeight: "600",
      color: "var(--foreground, inherit)",
    },
    {
      tag: [tags.processingInstruction, tags.punctuation],
      color: "var(--muted, #71717a)",
      opacity: "0.7",
    },
    {
      tag: tags.emphasis,
      fontStyle: "italic",
    },
    {
      tag: tags.strong,
      fontWeight: "600",
    },
    {
      tag: tags.link,
      color: "var(--accent, #006fee)",
      textDecoration: "underline",
      textUnderlineOffset: "3px",
    },
    {
      tag: tags.url,
      color: "var(--muted, #71717a)",
      opacity: "0.8",
    },
    {
      tag: tags.monospace,
      backgroundColor: "var(--default, rgba(0, 0, 0, 0.05))",
      borderRadius: "3px",
      padding: "1px 4px",
    },
    {
      tag: tags.quote,
      fontStyle: "italic",
      color: "var(--muted, #71717a)",
    },
    {
      tag: tags.list,
      color: "var(--accent, #006fee)",
    },
    {
      tag: tags.strikethrough,
      textDecoration: "line-through",
      opacity: "0.6",
    },
  ])
);
