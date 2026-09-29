import { describe, it, expect } from "bun:test";
import fs from "node:fs";
import { EXTENSION_CATALOG } from "../src/extensions/catalog";
import createCodeFilesExtension from "../src/extensions/bundles/codefiles";
import { EditorView } from "@codemirror/view";

const read = (p: string) => fs.readFileSync(new URL(p, import.meta.url), "utf-8");

describe("codefiles extension", () => {
  it("manifest correctness", () => {
    const manifest = EXTENSION_CATALOG.find((e) => e.id === "codefiles");
    expect(manifest).toBeDefined();
    expect(manifest?.fileExtensions).toBeDefined();
    // Doesn't include .json
    expect(manifest?.fileExtensions).not.toContain(".json");
    
    // Doesn't overlap with any other extension
    const otherExtensions = EXTENSION_CATALOG.filter((e) => e.id !== "codefiles");
    for (const other of otherExtensions) {
      if (!other.fileExtensions) continue;
      for (const ext of other.fileExtensions) {
        expect(manifest?.fileExtensions).not.toContain(ext);
      }
    }
  });

  it("resolves CodeMirror extensions", () => {
    const runtime = createCodeFilesExtension();
    expect(runtime.id).toBe("codefiles");
    
    const host = document.createElement("div");
    document.body.appendChild(host); // Need it in document for some DOM APIs
    const handle = runtime.mountFileEditor!(host, {
      initialValue: "foo",
      filePath: "test.yaml",
      onChange: () => {},
      onModeChange: () => {}
    });
    
    expect(handle).toBeDefined();
    handle.destroy();
    document.body.removeChild(host);
  });

  it("byte-for-byte pass-through", () => {
    const runtime = createCodeFilesExtension();
    const host = document.createElement("div");
    document.body.appendChild(host);
    
    let lastReportedText = "";
    const initialValue = "  \n\t mixed indent  \nno trailing newline";
    
    const handle = runtime.mountFileEditor!(host, {
      initialValue,
      filePath: "test.txt",
      onChange: (text) => { lastReportedText = text; },
      onModeChange: () => {}
    });

    // onChange shouldn't be invoked without edit
    expect(lastReportedText).toBe("");
    
    const editorEl = host.querySelector(".cm-editor") as HTMLElement;
    expect(editorEl).toBeTruthy();
    
    const view = EditorView.findFromDOM(editorEl);
    expect(view).toBeTruthy();
    
    // Simulate an edit
    view!.dispatch({
      changes: { from: 0, insert: "a" }
    });
    
    // Should pass through exactly what CodeMirror contains
    expect(lastReportedText).toBe("a" + initialValue);
    
    handle.destroy();
    document.body.removeChild(host);
  });

  it("contains no color literals in codefiles.css", () => {
    const source = read("../src/extensions/bundles/codefiles.css");
    const withoutComments = source.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(withoutComments.match(/#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(/gi)).toBeNull();
  });

  // The literal-color check above only scans the .css file. codefiles.ts sets colors from
  // JS too (its CodeMirror HighlightStyle), and a `var(--typo-d-token, #hexFallback)`
  // referencing a token that plain grepping never catches: it isn't a literal at the top
  // level, tsc doesn't know CSS custom properties exist, and it renders identically to a
  // real token in every case except that the fallback hex is all that ever shows, on every
  // theme and palette. (This is exactly how `--primary`/`--secondary` shipped here once —
  // real-looking, always falling back, undetected until checked in a real browser.) So
  // every `var(--x, ...)` in the extension's own source, CSS or JS, is checked against the
  // app's actual declared custom properties — its own tokens (App.css) plus HeroUI's base
  // theme, the same two places every other extension's real tokens come from.
  it("references only var(--token) names that are actually declared somewhere", () => {
    const appCss = fs.readFileSync(
      new URL("../src/App.css", import.meta.url),
      "utf-8",
    );
    const heroCss = fs.readFileSync(
      new URL("../node_modules/@heroui/styles/dist/themes/default/variables.css", import.meta.url),
      "utf-8",
    );
    const declared = new Set(
      [...`${appCss}\n${heroCss}`.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gm)].map((m) => m[1]),
    );

    const sources = [read("../src/extensions/bundles/codefiles.ts"), read("../src/extensions/bundles/codefiles.css")];
    const referenced = new Set(
      sources.flatMap((s) => [...s.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1])),
    );
    expect([...referenced].filter((name) => !declared.has(name)).sort()).toEqual([]);
  });
});
