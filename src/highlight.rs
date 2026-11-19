//! Syntax highlighting for the editor, built on `syntect` (pure-Rust regex).
//!
//! The language ids, fence aliases and extensions mirror what the web build
//! offers: code fences follow `components/Editor/codeLanguages.ts` and code files
//! follow `extensions/bundles/codefiles.ts`. Markdown source is a language too, and
//! its fenced blocks are coloured in the fence's own language.
//!
//! Highlighting is line-based and stateful (a string can open on one line and
//! close on a later one), so [`LineCache`] keeps parser checkpoints. A repaint
//! resumes from the nearest checkpoint above the visible lines, and an edit only
//! discards the checkpoints below the first byte that changed.
//!
//! Token ranges are byte offsets into the text, sorted and non-overlapping, on
//! char boundaries. Bytes no token covers (line breaks) are plain text.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use gpui::Hsla;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::theme::Theme;

/// Buffers above this size are painted without colour. Parsing is linear, so
/// this bounds the work a single repaint or edit can cost.
pub const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Operator,
    Punctuation,
    Property,
    Constant,
    Tag,
    Attribute,
    Heading,
    Emphasis,
    Strong,
    Link,
    Code,
    Quote,
    ListMarker,
}

struct Lang {
    /// The canonical id: what `highlight` and `Editor::set_language` take.
    id: &'static str,
    /// Code-fence names (lowercase), from `codeLanguages.ts` names and aliases.
    fences: &'static [&'static str],
    /// File extensions (lowercase), the union of `codeLanguages.ts` and `codefiles.ts`.
    exts: &'static [&'static str],
    /// syntect grammar names, most wanted first. Used when no extension resolves.
    /// syntect's default set lacks some languages, so the fallback is a close
    /// relative (TypeScript uses JavaScript, Kotlin uses Java).
    grammars: &'static [&'static str],
}

const LANGS: &[Lang] = &[
    Lang {
        id: "javascript",
        fences: &["ecmascript", "js", "node"],
        exts: &["js", "mjs", "cjs"],
        grammars: &["JavaScript"],
    },
    Lang {
        id: "typescript",
        fences: &["ts"],
        exts: &["ts", "mts", "cts"],
        grammars: &["TypeScript", "JavaScript"],
    },
    Lang {
        id: "jsx",
        fences: &[],
        exts: &["jsx"],
        grammars: &["JavaScript (Babel)", "JavaScript"],
    },
    Lang {
        id: "tsx",
        fences: &[],
        exts: &["tsx"],
        grammars: &["TypeScriptReact", "JavaScript"],
    },
    Lang {
        id: "json",
        fences: &["json5"],
        exts: &["json", "map"],
        grammars: &["JSON"],
    },
    Lang {
        id: "python",
        fences: &["py"],
        exts: &["py", "pyw"],
        grammars: &["Python"],
    },
    Lang {
        id: "html",
        fences: &["xhtml"],
        exts: &["html", "htm"],
        grammars: &["HTML"],
    },
    Lang {
        id: "css",
        fences: &[],
        exts: &["css"],
        grammars: &["CSS"],
    },
    Lang {
        id: "yaml",
        fences: &["yml"],
        exts: &["yaml", "yml"],
        grammars: &["YAML"],
    },
    Lang {
        id: "xml",
        fences: &["rss", "wsdl", "xsd"],
        exts: &["xml", "xsl", "xsd", "svg"],
        grammars: &["XML"],
    },
    Lang {
        id: "sql",
        fences: &[],
        exts: &["sql"],
        grammars: &["SQL"],
    },
    Lang {
        id: "rust",
        fences: &[],
        exts: &["rs"],
        grammars: &["Rust"],
    },
    Lang {
        id: "go",
        fences: &[],
        exts: &["go"],
        grammars: &["Go"],
    },
    Lang {
        id: "java",
        fences: &[],
        exts: &["java"],
        grammars: &["Java"],
    },
    Lang {
        id: "cpp",
        fences: &["c++"],
        exts: &[
            "cpp", "c++", "cc", "cxx", "hpp", "h++", "hh", "hxx", "c", "h",
        ],
        grammars: &["C++"],
    },
    Lang {
        id: "c",
        fences: &[],
        exts: &[],
        grammars: &["C"],
    },
    Lang {
        id: "php",
        fences: &[],
        exts: &["php", "phtml"],
        grammars: &["PHP"],
    },
    Lang {
        id: "markdown",
        fences: &["md"],
        exts: &["md", "markdown", "mkd"],
        grammars: &["Markdown"],
    },
    Lang {
        id: "shell",
        fences: &["bash", "sh", "zsh"],
        exts: &["sh", "ksh", "bash", "zsh"],
        grammars: &["Bourne Again Shell (bash)"],
    },
    Lang {
        id: "toml",
        fences: &[],
        exts: &["toml"],
        grammars: &["TOML", "Java Properties"],
    },
    Lang {
        id: "dockerfile",
        fences: &[],
        exts: &["dockerfile"],
        grammars: &["Dockerfile", "Bourne Again Shell (bash)"],
    },
    Lang {
        id: "ruby",
        fences: &["jruby", "macruby", "rake", "rb", "rbx"],
        exts: &["rb"],
        grammars: &["Ruby"],
    },
    Lang {
        id: "swift",
        fences: &[],
        exts: &["swift"],
        grammars: &["Swift", "Rust"],
    },
    Lang {
        id: "kotlin",
        fences: &[],
        exts: &["kt", "kts"],
        grammars: &["Kotlin", "Java"],
    },
    Lang {
        id: "csharp",
        fences: &["c#", "csharp", "cs"],
        exts: &["cs"],
        grammars: &["C#"],
    },
    Lang {
        id: "properties",
        fences: &["ini"],
        exts: &["properties", "ini", "in", "env"],
        grammars: &["Java Properties"],
    },
    Lang {
        id: "diff",
        fences: &["patch"],
        exts: &["diff", "patch"],
        grammars: &["Diff"],
    },
];

fn find_lang(name: &str) -> Option<&'static Lang> {
    let name = name.to_ascii_lowercase();
    LANGS
        .iter()
        .find(|lang| lang.id == name || lang.fences.contains(&name.as_str()))
}

/// The language a file is highlighted as, by file name. Files without a known
/// extension (`.gitignore`, `Makefile`) get `None` and stay plain.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    if base == "dockerfile" {
        return Some("dockerfile");
    }
    let (_, ext) = base.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    LANGS
        .iter()
        .find(|lang| lang.exts.contains(&ext))
        .map(|lang| lang.id)
}

/// The language named by a code fence's info string (the first word, any case).
pub fn language_for_fence(name: &str) -> Option<&'static str> {
    let word = name.split_whitespace().next()?;
    find_lang(word).map(|lang| lang.id)
}

/// Highlights a whole text. Unknown languages come back as one plain token.
pub fn highlight(language: &str, text: &str) -> Vec<(Range<usize>, TokenKind)> {
    if text.is_empty() {
        return Vec::new();
    }
    let Some(mut cache) = LineCache::new(language) else {
        return vec![(0..text.len(), TokenKind::Plain)];
    };
    let line_count = text.matches('\n').count() + 1;
    let mut out = Vec::new();
    let mut base = 0;
    for (line, tokens) in text.split('\n').zip(cache.lines(text, 0, line_count)) {
        for (range, kind) in tokens {
            out.push((base + range.start..base + range.end, kind));
        }
        base += line.len() + 1;
    }
    out
}

/// Parser state at the start of a line, so highlighting can resume there.
#[derive(Clone)]
struct Checkpoint {
    line: usize,
    offset: usize,
    parse: ParseState,
    stack: ScopeStack,
}

/// Saved parser state is at least this often, in lines.
const CHECKPOINT_EVERY: usize = 32;

/// How far above a wanted line the walk may start from a saved state. Beyond
/// this a fresh parser starts instead, so a jump costs a bounded amount of work.
const LOOKBACK_LINES: usize = 200;

/// Incremental highlighter for one document. Keep one per editor and call
/// [`LineCache::lines`] with the visible window on every repaint.
pub struct LineCache {
    syntax: &'static SyntaxReference,
    /// The text the checkpoints and line tokens were built from.
    source: String,
    /// Sorted by line; the first is always line 0.
    checkpoints: Vec<Checkpoint>,
    /// Tokens of the lines the editor has asked for, by line index.
    lines: HashMap<usize, Vec<(Range<usize>, TokenKind)>>,
    /// Memoised scope-to-kind lookups, so each scope name is resolved once.
    scope_kinds: HashMap<Scope, Option<TokenKind>>,
}

impl LineCache {
    /// `None` when the language is unknown.
    pub fn new(language: &str) -> Option<Self> {
        let lang = find_lang(language)?;
        let syntax = find_syntax(syntax_set(), lang);
        Some(Self {
            syntax,
            source: String::new(),
            checkpoints: vec![Checkpoint {
                line: 0,
                offset: 0,
                parse: ParseState::new(syntax),
                stack: ScopeStack::new(),
            }],
            lines: HashMap::new(),
            scope_kinds: HashMap::new(),
        })
    }

    /// Tokens for lines `first..last` of `text`. Each entry holds byte ranges
    /// relative to that line's start, excluding its newline.
    ///
    /// Work is proportional to the lines not seen before: a repaint of an
    /// unchanged window does no parsing, and a scroll parses only the lines it
    /// reveals.
    pub fn lines(
        &mut self,
        text: &str,
        first: usize,
        last: usize,
    ) -> Vec<Vec<(Range<usize>, TokenKind)>> {
        self.sync(text);
        if let Some(from) = (first..last).find(|line| !self.lines.contains_key(line)) {
            self.walk(text, from, last);
        }
        let out = (first..last)
            .map(|line| self.lines.get(&line).cloned().unwrap_or_default())
            .collect();
        // Forget lines well away from the window so scrolling does not grow the cache.
        let span = last.saturating_sub(first);
        if self.lines.len() > 3 * span + 64 {
            let keep = first.saturating_sub(span)..last.saturating_add(span);
            self.lines.retain(|line, _| keep.contains(line));
        }
        out
    }

    /// Parses from the nearest usable state up to `last`, keeping the tokens of
    /// lines `from..last` and saving checkpoints on the way.
    fn walk(&mut self, text: &str, from: usize, last: usize) {
        let nearest = self
            .checkpoints
            .iter()
            .rposition(|cp| cp.line <= from)
            .unwrap_or(0);
        let (mut line, mut offset, mut parse, mut stack) =
            if from - self.checkpoints[nearest].line <= LOOKBACK_LINES {
                let cp = &self.checkpoints[nearest];
                (cp.line, cp.offset, cp.parse.clone(), cp.stack.clone())
            } else {
                // Starts above the wanted lines with a fresh parser. Only a construct that
                // spans more than LOOKBACK_LINES lines can be coloured differently.
                let seed = from - LOOKBACK_LINES;
                (
                    seed,
                    line_start(text, seed),
                    ParseState::new(self.syntax),
                    ScopeStack::new(),
                )
            };

        while line < last && offset <= text.len() {
            let (content_end, next) = match text[offset..].find('\n') {
                Some(p) => (offset + p, offset + p + 1),
                None => (text.len(), text.len()),
            };
            let content_len = content_end - offset;
            let tokens = if next > content_end {
                self.line_tokens(&mut parse, &mut stack, &text[offset..next], content_len)
            } else {
                // The last line has no newline; syntect's grammars expect one.
                let with_newline = format!("{}\n", &text[offset..content_end]);
                self.line_tokens(&mut parse, &mut stack, &with_newline, content_len)
            };
            if line >= from {
                self.lines.insert(line, tokens);
            }
            line += 1;
            offset = next;
            let saves_here = line % CHECKPOINT_EVERY == 0 || line == last;
            if saves_here && self.checkpoints.last().is_none_or(|cp| cp.line < line) {
                self.checkpoints.push(Checkpoint {
                    line,
                    offset,
                    parse: parse.clone(),
                    stack: stack.clone(),
                });
            }
        }
    }

    /// Drops what a change can affect: checkpoints and line tokens from the
    /// first changed byte on.
    fn sync(&mut self, text: &str) {
        if self.source == text {
            return;
        }
        let prefix = self
            .source
            .bytes()
            .zip(text.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let changed_line = self.source.as_bytes()[..prefix]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        // A checkpoint at offset `o` depends only on `text[..o]`.
        self.checkpoints.retain(|cp| cp.offset <= prefix);
        self.lines.retain(|&line, _| line < changed_line);
        self.source.clear();
        self.source.push_str(text);
    }

    fn line_tokens(
        &mut self,
        parse: &mut ParseState,
        stack: &mut ScopeStack,
        line: &str,
        content_len: usize,
    ) -> Vec<(Range<usize>, TokenKind)> {
        let ops = parse.parse_line(line, syntax_set()).unwrap_or_default();
        let mut tokens = Vec::new();
        let mut pos = 0;
        for (index, op) in ops {
            if pos < index {
                let kind = self.kind_of(stack);
                push_token(&mut tokens, pos..index, kind, content_len);
            }
            let _ = stack.apply(&op);
            pos = index;
        }
        if pos < content_len {
            let kind = self.kind_of(stack);
            push_token(&mut tokens, pos..content_len, kind, content_len);
        }
        tokens
    }

    /// The kind of the innermost scope that maps to one. Punctuation (quote
    /// marks, brackets) only wins when no enclosing scope says more.
    fn kind_of(&mut self, stack: &ScopeStack) -> TokenKind {
        let mut punctuation = false;
        for scope in stack.scopes.iter().rev() {
            let kind = *self
                .scope_kinds
                .entry(*scope)
                .or_insert_with(|| kind_for_scope(&(*scope).build_string()));
            match kind {
                Some(TokenKind::Punctuation) => punctuation = true,
                Some(kind) => return kind,
                None => {}
            }
        }
        if punctuation {
            TokenKind::Punctuation
        } else {
            TokenKind::Plain
        }
    }
}

/// The byte offset where line `n` starts, or the end of `text` past the last line.
fn line_start(text: &str, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(n - 1)
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len())
}

fn push_token(
    tokens: &mut Vec<(Range<usize>, TokenKind)>,
    range: Range<usize>,
    kind: TokenKind,
    limit: usize,
) {
    let end = range.end.min(limit);
    if range.start >= end {
        return;
    }
    if let Some((last, last_kind)) = tokens.last_mut() {
        if *last_kind == kind && last.end == range.start {
            last.end = end;
            return;
        }
    }
    tokens.push((range.start..end, kind));
}

/// Scope prefixes, most specific first: the first entry that matches a scope
/// name decides its kind. A prefix matches whole dot-separated segments only.
const SCOPE_KINDS: &[(&str, TokenKind)] = &[
    ("comment", TokenKind::Comment),
    ("string.other.link", TokenKind::Link),
    ("string", TokenKind::String),
    ("constant.character.escape", TokenKind::String),
    ("constant.numeric", TokenKind::Number),
    ("constant.language", TokenKind::Constant),
    ("constant", TokenKind::Constant),
    ("keyword.operator", TokenKind::Operator),
    ("keyword", TokenKind::Keyword),
    ("storage", TokenKind::Keyword),
    ("variable.language", TokenKind::Keyword),
    ("entity.name.function", TokenKind::Function),
    ("support.function", TokenKind::Function),
    ("variable.function", TokenKind::Function),
    ("entity.name.tag", TokenKind::Tag),
    ("entity.other.attribute-name", TokenKind::Attribute),
    ("entity.name.type", TokenKind::Type),
    ("entity.name.class", TokenKind::Type),
    ("entity.other.inherited-class", TokenKind::Type),
    ("support.type.property-name", TokenKind::Property),
    ("support.type", TokenKind::Type),
    ("support.class", TokenKind::Type),
    ("support.constant", TokenKind::Constant),
    ("support.variable", TokenKind::Property),
    ("variable.other.constant", TokenKind::Constant),
    ("variable.other.member", TokenKind::Property),
    ("variable.other.property", TokenKind::Property),
    ("meta.property-name", TokenKind::Property),
    ("markup.heading", TokenKind::Heading),
    ("markup.bold", TokenKind::Strong),
    ("markup.italic", TokenKind::Emphasis),
    ("markup.inline.raw", TokenKind::Code),
    ("markup.raw", TokenKind::Code),
    ("markup.underline.link", TokenKind::Link),
    ("markup.quote", TokenKind::Quote),
    ("markup.list", TokenKind::ListMarker),
    ("punctuation.definition.list", TokenKind::ListMarker),
    ("markup.inserted", TokenKind::String),
    ("markup.deleted", TokenKind::Tag),
    ("punctuation", TokenKind::Punctuation),
];

fn kind_for_scope(name: &str) -> Option<TokenKind> {
    SCOPE_KINDS
        .iter()
        .find(|(prefix, _)| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .map(|(_, kind)| *kind)
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn find_syntax(set: &'static SyntaxSet, lang: &Lang) -> &'static SyntaxReference {
    lang.exts
        .iter()
        .find_map(|ext| set.find_syntax_by_extension(ext))
        .or_else(|| {
            lang.grammars
                .iter()
                .find_map(|name| set.find_syntax_by_name(name))
        })
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// Whether a token kind is drawn in bold or italic: `(bold, italic)`.
pub fn emphasis(kind: TokenKind) -> (bool, bool) {
    match kind {
        TokenKind::Heading | TokenKind::Strong => (true, false),
        TokenKind::Emphasis | TokenKind::Comment => (false, true),
        _ => (false, false),
    }
}

/// The colour a token kind is painted in, from the theme's palette.
pub fn color(kind: TokenKind, theme: &Theme) -> Hsla {
    let syntax = &theme.syntax;
    let color = match kind {
        TokenKind::Plain | TokenKind::Emphasis | TokenKind::Strong => theme.fg,
        TokenKind::Keyword => syntax.keyword,
        TokenKind::String => syntax.string,
        TokenKind::Number => syntax.number,
        TokenKind::Comment => syntax.comment,
        TokenKind::Function => syntax.function,
        TokenKind::Type => syntax.ty,
        TokenKind::Operator => syntax.operator,
        TokenKind::Punctuation => theme.muted,
        TokenKind::Property => syntax.property,
        TokenKind::Constant => syntax.constant,
        TokenKind::Tag => syntax.tag,
        TokenKind::Attribute => syntax.attribute,
        TokenKind::Heading => theme.heading,
        TokenKind::Link | TokenKind::ListMarker => theme.accent,
        TokenKind::Code => theme.code,
        TokenKind::Quote => theme.quote,
    };
    color.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_well_formed(text: &str, tokens: &[(Range<usize>, TokenKind)]) {
        let mut previous_end = 0;
        for (range, _) in tokens {
            assert!(range.start < range.end, "empty range {range:?}");
            assert!(
                range.start >= previous_end,
                "overlap or unsorted at {range:?}"
            );
            assert!(range.end <= text.len(), "past end {range:?}");
            assert!(
                text.is_char_boundary(range.start),
                "start mid-char {range:?}"
            );
            assert!(text.is_char_boundary(range.end), "end mid-char {range:?}");
            previous_end = range.end;
        }
    }

    fn kinds_of<'a>(
        text: &'a str,
        tokens: &[(Range<usize>, TokenKind)],
        kind: TokenKind,
    ) -> Vec<&'a str> {
        tokens
            .iter()
            .filter(|(_, k)| *k == kind)
            .map(|(r, _)| &text[r.clone()])
            .collect()
    }

    const THAI_RUST: &str =
        "// ความคิดเห็น ภาษาไทย\nfn ไก่() {\n    let s = \"สวัสดี\";\n    let n = 42;\n}\n";

    #[test]
    fn ranges_are_sorted_and_on_char_boundaries_for_every_language() {
        let samples: &[(&str, &str)] = &[
            ("rust", THAI_RUST),
            ("python", "def f(x):\n    return 'ไก่' + str(x)  # ความ\n\"\"\"doc\nสอง\"\"\"\n"),
            ("javascript", "const é = `${'ลาว'}`; // 注释\nexport default () => 1.5e3;\n"),
            ("typescript", "interface Ünïcode { a: string }\n\n"),
            ("json", "{\"ภาษา\": [1, 2.5, true, null]}"),
            ("markdown", "# หัวข้อ\n\n**ตัวหนา** *เอียง* `โค้ด`\n> คำพูด\n- รายการ\n1. ข้อหนึ่ง\n```rust\nfn x() {}\n```\n[ลิงก์](http://x)\n"),
            ("html", "<div class=\"ก\">ข<!-- ค --></div>\r\n"),
            ("yaml", "key: ค่า\nlist:\n  - ไทย\n"),
            ("sql", "SELECT 'ก', 1 FROM t -- หมายเหตุ\n"),
            ("nope", THAI_RUST),
        ];
        for (language, text) in samples {
            let tokens = highlight(language, text);
            assert_well_formed(text, &tokens);
        }
    }

    /// The per-line, line-relative view of a whole-text highlight, for comparing with windows.
    fn relative_lines(
        text: &str,
        full: &[(Range<usize>, TokenKind)],
        first: usize,
        last: usize,
    ) -> Vec<Vec<(Range<usize>, TokenKind)>> {
        let mut bounds = Vec::new();
        let mut base = 0;
        for line in text.split('\n') {
            bounds.push((base, line.len()));
            base += line.len() + 1;
        }
        (first..last)
            .map(|index| {
                let (base, len) = bounds[index];
                full.iter()
                    .filter(|(r, _)| r.start >= base && r.end <= base + len)
                    .map(|(r, kind)| (r.start - base..r.end - base, *kind))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn line_cache_windows_match_full_highlight() {
        let mut text = String::new();
        for i in 0..300 {
            if i == 100 {
                text.push_str("\"\"\"opening a string that spans lines\n");
            } else if i == 110 {
                text.push_str("closing ไก่ \"\"\" x = 1\n");
            } else {
                text.push_str(&format!("value_{i} = 'ภาษา{i}'  # note {i}\n"));
            }
        }
        let full = highlight("python", &text);
        assert_well_formed(&text, &full);

        let line_count = text.matches('\n').count() + 1;
        let mut cache = LineCache::new("python").expect("python is known");
        // Windows in an arbitrary order, including ones that reuse cached lines.
        for (first, last) in [
            (0, 20),
            (150, 160),
            (105, 115),
            (290, line_count),
            (0, line_count),
            (20, 40),
        ] {
            assert_eq!(
                cache.lines(&text, first, last),
                relative_lines(&text, &full, first, last),
                "window {first}..{last}"
            );
        }

        // An edit near the top must invalidate what sits below it.
        let edited = text.replacen("value_5 ", "value_5 = \"open", 1);
        let full_after = highlight("python", &edited);
        assert_well_formed(&edited, &full_after);
        assert_eq!(
            cache.lines(&edited, 100, 112),
            relative_lines(&edited, &full_after, 100, 112)
        );
        assert_eq!(
            cache.lines(&edited, 0, 12),
            relative_lines(&edited, &full_after, 0, 12)
        );
    }

    #[test]
    fn far_window_is_well_formed_without_walking_from_the_top() {
        let mut text = String::new();
        for i in 0..2000 {
            text.push_str(&format!("let v{i} = \"ไก่ {i}\"; // {i}\n"));
        }
        let mut cache = LineCache::new("rust").expect("rust is known");
        let lines = cache.lines(&text, 1900, 1930);
        assert_eq!(lines.len(), 30);
        for tokens in &lines {
            let mut previous = 0;
            for (range, _) in tokens {
                assert!(range.start >= previous);
                previous = range.end;
            }
        }
        assert!(lines
            .iter()
            .flatten()
            .any(|(_, k)| *k == TokenKind::Keyword));
    }

    #[test]
    fn rust_keywords_strings_and_comments() {
        let text = "fn main() {\n    let s = \"hi\"; // note\n}\n";
        let tokens = highlight("rust", text);
        assert_well_formed(text, &tokens);
        assert!(kinds_of(text, &tokens, TokenKind::Keyword).contains(&"fn"));
        assert!(kinds_of(text, &tokens, TokenKind::Keyword).contains(&"let"));
        assert!(kinds_of(text, &tokens, TokenKind::String)
            .iter()
            .any(|s| s.contains("hi")));
        assert!(kinds_of(text, &tokens, TokenKind::Comment)
            .iter()
            .any(|s| s.contains("note")));
    }

    #[test]
    fn python_and_json_tokens() {
        let text = "def f(x):\n    return 'a'  # c\n";
        let tokens = highlight("python", text);
        assert_well_formed(text, &tokens);
        assert!(kinds_of(text, &tokens, TokenKind::Keyword).contains(&"def"));
        assert!(kinds_of(text, &tokens, TokenKind::String)
            .iter()
            .any(|s| s.contains('a')));
        assert!(kinds_of(text, &tokens, TokenKind::Comment)
            .iter()
            .any(|s| s.contains('c')));

        let json = "{\"n\": 42, \"ok\": true}";
        let tokens = highlight("json", json);
        assert_well_formed(json, &tokens);
        assert!(kinds_of(json, &tokens, TokenKind::Number).contains(&"42"));
        assert!(kinds_of(json, &tokens, TokenKind::Constant).contains(&"true"));
        // syntect's JSON grammar scopes object keys as strings.
        assert!(kinds_of(json, &tokens, TokenKind::String).contains(&"\"n\""));
    }

    #[test]
    fn markdown_source_tokens() {
        let text = "# Title\n\n**bold** and *it* `code`\n> quoted\n- item\n";
        let tokens = highlight("markdown", text);
        assert_well_formed(text, &tokens);
        for kind in [
            TokenKind::Heading,
            TokenKind::Strong,
            TokenKind::Emphasis,
            TokenKind::Code,
            TokenKind::Quote,
            TokenKind::ListMarker,
        ] {
            assert!(tokens.iter().any(|(_, k)| *k == kind), "no {kind:?} token");
        }
    }

    #[test]
    fn unknown_language_is_one_plain_token() {
        let text = "ไก่ line\nnext";
        assert_eq!(
            highlight("brainfuck", text),
            vec![(0..text.len(), TokenKind::Plain)]
        );
        assert!(highlight("rust", "").is_empty());
    }

    #[test]
    fn every_language_has_a_real_grammar() {
        let set = syntax_set();
        let plain = set.find_syntax_plain_text().name.clone();
        let missing: Vec<&str> = LANGS
            .iter()
            .filter(|lang| find_syntax(set, lang).name == plain)
            .map(|lang| lang.id)
            .collect();
        assert!(missing.is_empty(), "no grammar for {missing:?}");
    }

    #[test]
    fn typescript_uses_a_javascript_family_grammar() {
        let text = "const n: number = 1;\n";
        let tokens = highlight("typescript", text);
        assert_well_formed(text, &tokens);
        assert!(kinds_of(text, &tokens, TokenKind::Keyword).contains(&"const"));
    }

    #[test]
    fn path_mapping() {
        assert_eq!(language_for_path("src/main.rs"), Some("rust"));
        assert_eq!(language_for_path("notes/App.TSX"), Some("tsx"));
        assert_eq!(language_for_path("C:\\proj\\config.yml"), Some("yaml"));
        assert_eq!(language_for_path("/x/Dockerfile"), Some("dockerfile"));
        assert_eq!(language_for_path("api.dockerfile"), Some("dockerfile"));
        assert_eq!(language_for_path("site/.env"), Some("properties"));
        assert_eq!(language_for_path("pkg.cfg/settings.toml"), Some("toml"));
        assert_eq!(language_for_path("a.d/Makefile"), None);
        assert_eq!(language_for_path(".gitignore"), None);
        assert_eq!(language_for_path("README"), None);
        assert_eq!(language_for_path("Welcome.md"), Some("markdown"));
        assert_eq!(language_for_path("logo.svg"), Some("xml"));
        assert_eq!(language_for_path("main.c"), Some("cpp"));
    }

    #[test]
    fn fence_mapping() {
        assert_eq!(language_for_fence("ts"), Some("typescript"));
        assert_eq!(language_for_fence("TypeScript"), Some("typescript"));
        assert_eq!(language_for_fence("js title=\"x\""), Some("javascript"));
        assert_eq!(language_for_fence("C++"), Some("cpp"));
        assert_eq!(language_for_fence("bash"), Some("shell"));
        assert_eq!(language_for_fence("yml"), Some("yaml"));
        // The first word decides, so the name "Properties files" resolves by its first word.
        assert_eq!(language_for_fence("Properties files"), Some("properties"));
        assert_eq!(language_for_fence("properties"), Some("properties"));
        assert_eq!(language_for_fence("c#"), Some("csharp"));
        assert_eq!(language_for_fence("text"), None);
        assert_eq!(language_for_fence("   "), None);
        assert_eq!(language_for_fence(""), None);
    }
}
