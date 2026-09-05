import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
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
      // Our own theme paints the editor; the packaged "light" one hardcodes a white background.
      theme="none"
      autoFocus
      className="h-full"
    />
  );
}
