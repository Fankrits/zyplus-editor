import { EditorView } from "@codemirror/view";
import { Extension } from "@codemirror/state";
import { HighlightStyle, syntaxHighlighting, StreamLanguage } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import { yaml } from "@codemirror/lang-yaml";
import { xml } from "@codemirror/lang-xml";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { go } from "@codemirror/lang-go";
import { sql } from "@codemirror/lang-sql";
import { php } from "@codemirror/lang-php";
import { cpp } from "@codemirror/lang-cpp";
import { java } from "@codemirror/lang-java";

import { toml } from "@codemirror/legacy-modes/mode/toml";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { kotlin } from "@codemirror/legacy-modes/mode/clike";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";

import type { ExtensionRuntime } from "../types";

// Syntax highlighting theme using HeroUI CSS variables to match plainTextTheme's light/dark agnostic approach
const codeHighlighting = syntaxHighlighting(
  HighlightStyle.define([
    // `--primary`/`--secondary` were never real tokens (grepped App.css and HeroUI's own
    // variables.css — neither defines them), so those two rules always fell through to a
    // hardcoded light-theme hex and never followed dark mode or a palette. `--accent` is
    // the real equivalent, and reusing it for more than one category matches
    // plainTextTheme.ts's own precedent (it colors both links and lists with --accent).
    { tag: [tags.keyword, tags.modifier], color: "var(--accent, #006fee)", fontWeight: "600" },
    { tag: [tags.string, tags.special(tags.string)], color: "var(--success, #17c964)" },
    { tag: [tags.number, tags.bool], color: "var(--warning, #f5a524)" },
    { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--muted, #71717a)", fontStyle: "italic" },
    { tag: [tags.variableName, tags.propertyName], color: "var(--foreground, inherit)" },
    { tag: [tags.typeName, tags.className], color: "var(--accent, #006fee)" },
    { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: "var(--accent, #006fee)" },
    { tag: tags.meta, color: "var(--muted, #71717a)" },
    { tag: tags.punctuation, color: "var(--muted, #71717a)", opacity: "0.7" },
  ])
);

function getLanguageExtension(filePath: string): Extension {
  // By file name, not by whatever follows the last dot in the whole path: a bare
  // `Dockerfile` has no extension, and `a.d/Makefile` must not read as ".d/Makefile".
  const base = (filePath.split(/[\\/]/).pop() ?? "").toLowerCase();
  const dot = base.lastIndexOf(".");
  const ext = base === "dockerfile" ? ".dockerfile" : dot >= 0 ? base.slice(dot) : "";
  switch (ext) {
    case ".yaml":
    case ".yml":
      return yaml();
    case ".toml":
      return StreamLanguage.define(toml);
    case ".xml":
      return xml();
    case ".sh":
    case ".bash":
    case ".zsh":
      return StreamLanguage.define(shell);
    case ".ini":
    case ".env":
      return StreamLanguage.define(properties);
    case ".css":
      return css();
    case ".html":
      return html();
    case ".js":
    case ".jsx":
      return javascript({ jsx: true });
    case ".ts":
    case ".tsx":
      return javascript({ jsx: true, typescript: true });
    case ".py":
      return python();
    case ".rs":
      return rust();
    case ".go":
      return go();
    case ".sql":
      return sql();
    case ".rb":
      return StreamLanguage.define(ruby);
    case ".php":
      return php();
    case ".c":
    case ".cpp":
    case ".h":
      return cpp();
    case ".java":
      return java();
    case ".kt":
      return StreamLanguage.define(kotlin);
    case ".swift":
      return StreamLanguage.define(swift);
    case ".dockerfile":
      return StreamLanguage.define(dockerFile);
    default:
      return []; // plain text fallback (e.g. .gitignore)
  }
}

export default function createCodeFilesExtension(): ExtensionRuntime {
  return {
    id: "codefiles",
    mountFileEditor(host, { initialValue, onChange, filePath }) {
      host.classList.add("codefiles-editor");

      const extensions = [
        getLanguageExtension(filePath || ""),
        EditorView.lineWrapping,
        codeHighlighting,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChange(update.state.doc.toString());
          }
        }),
      ];

      const editor = new EditorView({
        doc: initialValue,
        extensions,
        parent: host,
      });

      return {
        destroy() {
          editor.destroy();
          host.classList.remove("codefiles-editor");
        },
      };
    },
  };
}
