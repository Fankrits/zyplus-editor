# Plain Text Editor Refinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refine the Plain Text mode in zyplus-editor to provide a clean, distraction-free markdown editing canvas with centered layout, modern monospace typography, and subtle syntax styling.

**Architecture:** Create a modular CodeMirror theme and highlight style in `plainTextTheme.ts` using `@codemirror/view` and `@codemirror/language`, then apply them along with line wrapping and focus handlers in `PlainTextEditor.tsx`.

**Tech Stack:** React 19, TypeScript, CodeMirror 6 (`@uiw/react-codemirror`, `@codemirror/view`, `@codemirror/language`, `@codemirror/lang-markdown`, `@lezer/highlight`), HeroUI design tokens, Bun.

## Global Constraints

- Retain Bun as package manager and runtime.
- Use existing dependencies; do not add unnecessary external packages.
- Center content to max-width `48rem` (768px) with `3rem 1.5rem 12rem 1.5rem` padding.
- Eliminate focus outlines and distracting line bands.
- Clean commit per task with clear commit messages.

---

### Task 1: Create CodeMirror theme and markdown highlight style

**Files:**
- Create: `src/components/Editor/plainTextTheme.ts`

**Interfaces:**
- Produces:
  - `plainTextTheme: Extension` (EditorView.theme configuring centered layout, fonts, no outline, scroll-past-end)
  - `plainTextHighlighting: Extension` (syntaxHighlighting with HighlightStyle for muted markdown syntax tokens)

- [ ] **Step 1: Write `src/components/Editor/plainTextTheme.ts`**
Create `plainTextTheme.ts` exporting `plainTextTheme` and `plainTextHighlighting`.

- [ ] **Step 2: Verify compilation and type checking**
Run `bun run build` to ensure all imports and CodeMirror extension types resolve cleanly.

- [ ] **Step 3: Commit Task 1**
```bash
git add src/components/Editor/plainTextTheme.ts
git commit -m "feat(editor): create distraction-free plain text theme and highlight style"
```

---

### Task 2: Integrate theme and layout into PlainTextEditor

**Files:**
- Modify: `src/components/Editor/PlainTextEditor.tsx`

**Interfaces:**
- Consumes:
  - `plainTextTheme`, `plainTextHighlighting` from `src/components/Editor/plainTextTheme.ts`
- Produces:
  - `PlainTextEditor({ initialValue, onChange }: PlainTextEditorProps): JSX.Element`

- [ ] **Step 1: Update `src/components/Editor/PlainTextEditor.tsx`**
Replace previous inline theme and padding with `plainTextTheme` and `plainTextHighlighting`. Ensure full-height scroller and click-to-focus on the editor canvas.

- [ ] **Step 2: Verify build**
Run: `bun run build`
Expected: PASS with 0 errors.

- [ ] **Step 3: Commit Task 2**
```bash
git add src/components/Editor/PlainTextEditor.tsx
git commit -m "feat(editor): apply refined plain text theme to PlainTextEditor"
```

---

### Task 3: Visual & Functional Verification with agent-browser

**Files:**
- Verify: Running Vite dev server and checking Plain Text mode rendering via `agent-browser`.

- [ ] **Step 1: Start dev server in background if not running**
Verify that Vite dev server runs.

- [ ] **Step 2: Inspect Plain Text mode in browser**
Use agent-browser to navigate to the app, verify that:
1. Markdown text is centered within a 768px column.
2. Monospace font is used with clean line-height.
3. No harsh focus ring or distracting active line gutter appears.
4. Markdown tokens are cleanly and subtly styled.

- [ ] **Step 3: Commit any necessary refinements or test artifacts**
```bash
git commit --allow-empty -m "test(editor): verify refined plain text mode visually"
```
