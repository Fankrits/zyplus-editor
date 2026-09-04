# Plain Text Editor Refinement - Design Specification

- **Date:** 2026-09-05
- **Status:** Approved
- **Topic:** Distraction-Free Plain Text Markdown Editor

## 1. Overview
The goal of this enhancement is to refine the Plain Text mode in `zyplus-editor` into a distraction-free, modern, clean writing canvas. Currently, Plain Text mode renders with a hardcoded large padding and default CodeMirror styling (monospace font, raw unstyled markdown, harsh focus outlines, lack of centered layout). This design introduces a centered reading/writing column, modern monospace typography, clean editor chrome, and subtle markdown syntax highlighting.

## 2. Layout & Container
- **Centered Document Canvas:**
  - Content container (`.cm-content`, `.cm-line`) centered with `max-width: 48rem` (768px) and `margin: 0 auto`.
  - Responsive padding: `3rem 1.5rem 12rem 1.5rem` (48px top, 24px sides, 192px bottom).
  - Generous bottom scroll-padding ensures the user can continue typing at comfortable eye level ("scroll past end") rather than being pinned to the bottom border.
  - Clicking anywhere in the editor pane focuses the CodeMirror editor.
- **Scroll & Overflow:**
  - `EditorView.lineWrapping` enabled so lines do not overflow horizontally or cause awkward scrollbars.
  - Vertical scroll enabled with clean scrollbars matching the app style.

## 3. Typography & Styling
- **Font Stack:**
  - Font family: `ui-monospace, "SF Mono", Menlo, Monaco, Consolas, "JetBrains Mono", monospace`.
  - Font size: `15px`.
  - Line height: `1.65` (~`25px`).
  - Letter spacing: `-0.01em`.
- **Chrome & Interactivity:**
  - Focus outlines eliminated (`&.cm-focused { outline: none; }`).
  - Active line highlight removed or subdued to transparent to prevent distracting grey bands.
  - Selection background mapped to theme accent soft (`var(--accent-soft, rgba(0, 111, 238, 0.15))`).
  - Caret color matched to theme accent or primary text (`var(--accent, currentColor)`).

## 4. Subtle Markdown Syntax Highlighting
Using CodeMirror's `@codemirror/language` `syntaxHighlighting` and `HighlightStyle` along with `@lezer/highlight` tags:
- **Headings (`tags.heading`):** Font weight 600, subtle text emphasis.
- **Syntax delimiters (`tags.processingInstruction`, `tags.punctuation`):** Muted opacity (`var(--muted, #71717a)`) so characters like `#`, `*`, `_`, `` ` `` don't add visual clutter.
- **Emphasis / Strong (`tags.emphasis`, `tags.strong`):** Italic and bold styling applied directly to the emphasized content.
- **Inline Code & Code Blocks (`tags.monospace`):** Subtle background tint (`var(--default-100, rgba(0, 0, 0, 0.05))`) with light border radius.
- **Links (`tags.link`, `tags.url`):** Accent color for link text (`var(--accent)`), muted styling for destination URLs.
- **Quotes (`tags.quote`):** Muted italic styling.

## 5. Integration
- Replaces minimal styling in `src/components/Editor/PlainTextEditor.tsx`.
- Leverages existing `@uiw/react-codemirror`, `@codemirror/lang-markdown`, `@codemirror/view`, and imports `@codemirror/language` / `@lezer/highlight` (available via `@codemirror/lang-markdown` / `@uiw/react-codemirror`).
- Preserves two-way state synchronization (`onChange`, `initialValue`) with `workspaceStore`.
