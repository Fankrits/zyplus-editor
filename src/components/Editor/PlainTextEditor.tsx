import { useEffect, useMemo, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { search } from "@codemirror/search";
import { plainTextTheme, plainTextHighlighting } from "./plainTextTheme";
import { supportedLanguages } from "./codeBlockPreview";
import { createChangeReporter } from "./changeReporter";
import { registerPendingEdits } from "../../lib/pendingEdits";
import { useLatest } from "../../lib/useLatest";

interface PlainTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

// Without line wrapping, long lines overflow and force a horizontal
// scrollbar whose unstyled corner shows up as a stray black square.
const baseExtensions = [
  markdown({ codeLanguages: supportedLanguages }),
  // State only — our own SearchBar drives it, so CodeMirror's stock panel stays out.
  search(),
  EditorView.lineWrapping,
  plainTextTheme,
  plainTextHighlighting,
];

export function PlainTextEditor({ initialValue, onChange }: PlainTextEditorProps) {
  const onChangeRef = useLatest(onChange);
  // The editor reads its text once, at mount (EditorPane remounts it when the file changes
  // underneath). Passing the live store text back in would make CodeMirror compare it to the
  // document on every render, and a batched edit makes the two differ for a moment.
  const [initial] = useState(initialValue);

  const { reporter, extensions } = useMemo(() => {
    const reporter = createChangeReporter((text) => onChangeRef.current(text));
    // Our own listener, not the wrapper's `onChange`: that one copies the whole document out
    // on every change, before we can decide whether a report is due.
    const listener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        reporter.docChanged(update.state.doc.length, update.view);
      }
    });
    return { reporter, extensions: [...baseExtensions, listener] };
  }, [onChangeRef]);

  useEffect(() => {
    const unregister = registerPendingEdits(reporter);
    return () => {
      unregister();
      // Unmounting (another tab, the other editor) must not drop edits still waiting.
      reporter.flush();
    };
  }, [reporter]);

  return (
    <CodeMirror
      value={initial}
      extensions={extensions}
      basicSetup={{
        searchKeymap: false,
        lineNumbers: false,
        foldGutter: false,
        highlightActiveLineGutter: false,
        highlightActiveLine: false,
      }}
      height="100%"
      // Our own theme paints the editor; the packaged "light" one hardcodes a white background.
      theme="none"
      autoFocus
      className="h-full"
    />
  );
}

/** The editor's CodeMirror instance, found through the DOM so callers need no ref plumbing. */
export function findPlainEditor(): EditorView | null {
  const el = document.querySelector<HTMLElement>(".cm-editor");
  return el ? (EditorView.findFromDOM(el) ?? null) : null;
}
