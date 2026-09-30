import {
  createJSONEditor,
  isTextContent,
  jmespathQueryLanguage,
  jsonpathQueryLanguage,
  jsonQueryLanguage,
  Mode,
  type Content,
  type MenuItem,
} from "vanilla-jsoneditor";
import { parse, stringify } from "lossless-json";
import type { ExtensionRuntime } from "../types";

// Native JSON turns 12345678901234567890 into 12345678901234567000 and 1.0 into 1
// the first time a value is edited in the tree. Files come from anywhere, and
// IDs past 2^53 are common in them, so numbers are kept as written.
const parser = { parse, stringify };

// Lodash and JavaScript queries run through `new Function`, which the app's
// production CSP (rightly) blocks. Only the safe languages are offered.
const queryLanguages = [jsonQueryLanguage, jmespathQueryLanguage, jsonpathQueryLanguage];

function isValidJson(text: string): boolean {
  try {
    parser.parse(text);
    return true;
  } catch {
    return false;
  }
}

export default function createJsonExtension(): ExtensionRuntime {
  return {
    id: "json",
    mountFileEditor(host, { initialValue, onChange, mode, onModeChange }) {
      // Keep the file's own indentation and final newline so opening a file and
      // changing one value does not rewrite every line of it in version control.
      // The first indented line of any kind — an array of numbers or a minified-then-
      // pretty file has no quoted key to anchor on. Line endings survive too: a CRLF
      // file would otherwise come back all LF, a diff on every line.
      const indentation = /^(\t| +)\S/m.exec(initialValue)?.[1] ?? 2;
      const crlf = initialValue.includes("\r\n");
      const eol = /\n$/.test(initialValue) ? (crlf ? "\r\n" : "\n") : "";
      const serialize = (json: unknown) => {
        const text = stringify(json, null, indentation) ?? "";
        return (crlf ? text.replace(/\n/g, "\r\n") : text) + eol;
      };

      // The tab bar's choice if there is one; else a tree, unless there is none to show
      // (an empty or half-written file).
      let current = (mode as Mode | undefined) ?? (isValidJson(initialValue) ? Mode.tree : Mode.text);
      onModeChange(current);

      const editor = createJSONEditor({
        target: host,
        props: {
          content: { text: initialValue },
          mode: current,
          // The library can switch by itself too ("open in text mode" on invalid JSON).
          onChangeMode: (next: Mode) => {
            current = next;
            onModeChange(next);
          },
          // Choosing the view is the tab bar's job (`fileModes`), like Rich / Plain, so the
          // editor's own switch is dropped, along with the separator that followed it.
          onRenderMenu: (items: MenuItem[]) => {
            const rest = items.filter((i) => !(i.type === "button" && i.className?.includes("jse-group-button")));
            return rest[0]?.type === "separator" ? rest.slice(1) : rest;
          },
          parser,
          indentation,
          queryLanguages,
          queryLanguageId: jsonQueryLanguage.id,
          // Tree edits arrive as { json, text: undefined }, so `"text" in content` is not the test.
          onChange: (content: Content) =>
            onChange(isTextContent(content) ? content.text : serialize(content.json)),
        },
      });

      return {
        destroy() {
          void editor.destroy();
        },
        setMode(next) {
          if (next === current) return;
          current = next as Mode;
          void editor.updateProps({ mode: current });
        },
      };
    },
  };
}
