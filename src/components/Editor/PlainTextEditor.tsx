import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";

interface PlainTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

// Matches RichTextEditor's content offset: its px-8 py-6 wrapper (32px/24px)
// plus Crepe's built-in .ProseMirror padding (60px 120px).
const theme = EditorView.theme({
  "&": { fontSize: "16px" },
  ".cm-content": { padding: "84px 152px" },
});

// Without line wrapping, long lines overflow and force a horizontal
// scrollbar whose unstyled corner shows up as a stray black square.
const extensions = [markdown(), EditorView.lineWrapping, theme];

export function PlainTextEditor({ initialValue, onChange }: PlainTextEditorProps) {
  return (
    <CodeMirror
      value={initialValue}
      onChange={onChange}
      extensions={extensions}
      basicSetup={{ lineNumbers: false, foldGutter: false, highlightActiveLineGutter: false }}
      height="100%"
      autoFocus
      className="h-full"
    />
  );
}
