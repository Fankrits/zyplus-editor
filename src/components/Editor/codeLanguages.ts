import { LanguageDescription, LanguageSupport, StreamLanguage, type StreamParser } from "@codemirror/language";

const legacy = (parser: StreamParser<unknown>) => new LanguageSupport(StreamLanguage.define(parser));

/**
 * The languages a code fence is highlighted in. `@codemirror/language-data` offers about
 * 110, each its own chunk: together a third of what the desktop binary embeds, for
 * languages almost nobody writes in a note. Anything not listed still gets a code block,
 * in plain monospace, exactly like a language the registry never heard of.
 *
 * Names and aliases follow language-data's, so fences written for it keep matching.
 * Each `load` is a dynamic import, so a language costs nothing until a fence uses it.
 */
export const codeLanguages: LanguageDescription[] = [
  LanguageDescription.of({
    name: "JavaScript",
    alias: ["ecmascript", "js", "node"],
    extensions: ["js", "mjs", "cjs"],
    load: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  }),
  LanguageDescription.of({
    name: "TypeScript",
    alias: ["ts"],
    extensions: ["ts", "mts", "cts"],
    load: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true })),
  }),
  LanguageDescription.of({
    name: "JSX",
    extensions: ["jsx"],
    load: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ jsx: true })),
  }),
  LanguageDescription.of({
    name: "TSX",
    extensions: ["tsx"],
    load: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ jsx: true, typescript: true })),
  }),
  LanguageDescription.of({
    name: "JSON",
    alias: ["json5"],
    extensions: ["json", "map"],
    load: () => import("@codemirror/lang-json").then((m) => m.json()),
  }),
  LanguageDescription.of({
    name: "Python",
    alias: ["py"],
    extensions: ["py", "pyw"],
    load: () => import("@codemirror/lang-python").then((m) => m.python()),
  }),
  LanguageDescription.of({
    name: "HTML",
    alias: ["xhtml"],
    extensions: ["html", "htm"],
    load: () => import("@codemirror/lang-html").then((m) => m.html()),
  }),
  LanguageDescription.of({
    name: "CSS",
    extensions: ["css"],
    load: () => import("@codemirror/lang-css").then((m) => m.css()),
  }),
  LanguageDescription.of({
    name: "YAML",
    alias: ["yml"],
    extensions: ["yaml", "yml"],
    load: () => import("@codemirror/lang-yaml").then((m) => m.yaml()),
  }),
  LanguageDescription.of({
    name: "XML",
    alias: ["rss", "wsdl", "xsd"],
    extensions: ["xml", "xsl", "xsd", "svg"],
    load: () => import("@codemirror/lang-xml").then((m) => m.xml()),
  }),
  LanguageDescription.of({
    name: "SQL",
    extensions: ["sql"],
    load: () => import("@codemirror/lang-sql").then((m) => m.sql()),
  }),
  LanguageDescription.of({
    name: "Rust",
    extensions: ["rs"],
    load: () => import("@codemirror/lang-rust").then((m) => m.rust()),
  }),
  LanguageDescription.of({
    name: "Go",
    extensions: ["go"],
    load: () => import("@codemirror/lang-go").then((m) => m.go()),
  }),
  LanguageDescription.of({
    name: "Java",
    extensions: ["java"],
    load: () => import("@codemirror/lang-java").then((m) => m.java()),
  }),
  LanguageDescription.of({
    name: "C++",
    alias: ["cpp"],
    extensions: ["cpp", "c++", "cc", "cxx", "hpp", "h++", "hh", "hxx"],
    load: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  }),
  LanguageDescription.of({
    name: "C",
    extensions: ["c", "h"],
    load: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  }),
  LanguageDescription.of({
    name: "PHP",
    extensions: ["php", "phtml"],
    load: () => import("@codemirror/lang-php").then((m) => m.php()),
  }),
  LanguageDescription.of({
    name: "Markdown",
    alias: ["md"],
    extensions: ["md", "markdown", "mkd"],
    load: () => import("@codemirror/lang-markdown").then((m) => m.markdown()),
  }),
  LanguageDescription.of({
    name: "Shell",
    alias: ["bash", "sh", "zsh"],
    extensions: ["sh", "ksh", "bash"],
    load: () => import("@codemirror/legacy-modes/mode/shell").then((m) => legacy(m.shell)),
  }),
  LanguageDescription.of({
    name: "TOML",
    extensions: ["toml"],
    load: () => import("@codemirror/legacy-modes/mode/toml").then((m) => legacy(m.toml)),
  }),
  LanguageDescription.of({
    name: "Dockerfile",
    filename: /^Dockerfile$/,
    load: () => import("@codemirror/legacy-modes/mode/dockerfile").then((m) => legacy(m.dockerFile)),
  }),
  LanguageDescription.of({
    name: "Ruby",
    alias: ["jruby", "macruby", "rake", "rb", "rbx"],
    extensions: ["rb"],
    load: () => import("@codemirror/legacy-modes/mode/ruby").then((m) => legacy(m.ruby)),
  }),
  LanguageDescription.of({
    name: "Swift",
    extensions: ["swift"],
    load: () => import("@codemirror/legacy-modes/mode/swift").then((m) => legacy(m.swift)),
  }),
  LanguageDescription.of({
    name: "Kotlin",
    extensions: ["kt", "kts"],
    load: () => import("@codemirror/legacy-modes/mode/clike").then((m) => legacy(m.kotlin)),
  }),
  LanguageDescription.of({
    name: "C#",
    alias: ["csharp", "cs"],
    extensions: ["cs"],
    load: () => import("@codemirror/legacy-modes/mode/clike").then((m) => legacy(m.csharp)),
  }),
  LanguageDescription.of({
    name: "Properties files",
    alias: ["ini", "properties"],
    extensions: ["properties", "ini", "in"],
    load: () => import("@codemirror/legacy-modes/mode/properties").then((m) => legacy(m.properties)),
  }),
  LanguageDescription.of({
    name: "Diff",
    alias: ["patch"],
    extensions: ["diff", "patch"],
    load: () => import("@codemirror/legacy-modes/mode/diff").then((m) => legacy(m.diff)),
  }),
];
