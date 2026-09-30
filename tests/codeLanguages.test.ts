import { describe, it, expect } from "bun:test";
import { LanguageDescription, LanguageSupport } from "@codemirror/language";
import { supportedLanguages } from "../src/components/Editor/codeBlockPreview";
import { codeLanguages } from "../src/components/Editor/codeLanguages";

// The fence names people actually write, as they resolve for both editors' code blocks.
const FENCES: Record<string, string> = {
  js: "JavaScript", javascript: "JavaScript", node: "JavaScript",
  ts: "TypeScript", typescript: "TypeScript", jsx: "JSX", tsx: "TSX",
  json: "JSON", py: "Python", python: "Python", html: "HTML", css: "CSS",
  yml: "YAML", yaml: "YAML", xml: "XML", sql: "SQL", rust: "Rust", go: "Go", java: "Java",
  cpp: "C++", "c++": "C++", c: "C", php: "PHP", md: "Markdown", markdown: "Markdown",
  sh: "Shell", bash: "Shell", zsh: "Shell", toml: "TOML", ruby: "Ruby", rb: "Ruby",
  swift: "Swift", kotlin: "Kotlin", csharp: "C#", cs: "C#", ini: "Properties files", diff: "Diff",
  mermaid: "Mermaid",
};

describe("code block languages", () => {
  it("resolves every fence name people use", () => {
    for (const [fence, name] of Object.entries(FENCES)) {
      expect(LanguageDescription.matchLanguageName(supportedLanguages, fence, true)?.name).toBe(name);
    }
  });

  it("loads every language it lists", async () => {
    for (const lang of codeLanguages) {
      expect(await lang.load()).toBeInstanceOf(LanguageSupport);
    }
  });

  it("leaves a language it does not list unmatched, which renders as plain code", () => {
    expect(LanguageDescription.matchLanguageName(supportedLanguages, "haskell", true)).toBeNull();
  });
});
