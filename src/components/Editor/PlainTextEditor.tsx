import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";

interface PlainTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

const extensions = [markdown()];

export function PlainTextEditor({ initialValue, onChange }: PlainTextEditorProps) {
  return (
    <CodeMirror
      value={initialValue}
      onChange={onChange}
      extensions={extensions}
      height="100%"
      autoFocus
      className="h-full text-sm"
    />
  );
}
