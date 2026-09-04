import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { plainTextTheme, plainTextHighlighting } from "./plainTextTheme";

interface PlainTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

// Without line wrapping, long lines overflow and force a horizontal
// scrollbar whose unstyled corner shows up as a stray black square.
const extensions = [
  markdown(),
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
        lineNumbers: false,
        foldGutter: false,
        highlightActiveLineGutter: false,
        highlightActiveLine: false,
      }}
      height="100%"
      autoFocus
      className="h-full"
    />
  );
}
