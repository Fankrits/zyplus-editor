import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { search } from "@codemirror/search";
import { plainTextTheme, plainTextHighlighting } from "./plainTextTheme";
import { supportedLanguages } from "./codeBlockPreview";

interface PlainTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

// Without line wrapping, long lines overflow and force a horizontal
// scrollbar whose unstyled corner shows up as a stray black square.
const extensions = [
  markdown({ codeLanguages: supportedLanguages }),
  // State only — our own SearchBar drives it, so CodeMirror's stock panel stays out.
  search(),
  EditorView.lineWrapping,
  plainTextTheme,
  plainTextHighlighting,
];

export function PlainTextEditor({ initialValue, onChange }: PlainTextEditorProps) {
  return (
    <CodeMirror
      value={initialValue}
      onChange={onChange}
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
