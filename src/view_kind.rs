//! Which view a file opens in, and which modes each view offers.
//!
//! This is the pure half of the tab kinds: it maps a path and the extension
//! switches to a [`ViewKind`], and turns persisted mode ids into the ids the
//! views understand. It has no GPUI dependency so it can be unit-tested.
//!
//! Persisted mode ids: `rich`/`plain` for markdown, `text`/`tree`/`table` for
//! JSON, and `table`/`text` for CSV.

use crate::highlight;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    /// Markdown and plain text: a rendered preview (`rich`) or the editor (`plain`).
    /// `language` colours the editor; `None` for `.txt`.
    Markdown { language: Option<&'static str> },
    /// Any other file: the editor only, coloured as `language` when known.
    Text { language: Option<&'static str> },
    /// `.csv` / `.tsv` with the CSV extension on.
    Csv,
    /// `.json` with the JSON extension on.
    Json,
}

/// The lowercase extension of a file name, when it has one that is not the
/// whole name (`.env` and `Makefile` have none).
fn extension(path: &str) -> Option<String> {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (stem, ext) = base.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// The view a file opens in. `enabled` answers whether a built-in extension
/// id (`csv`, `json`, `codefiles`) is on in Settings.
pub fn kind_for_path(path: &str, enabled: impl Fn(&str) -> bool) -> ViewKind {
    let ext = extension(path);
    match ext.as_deref() {
        Some("md" | "markdown" | "mkd") => ViewKind::Markdown {
            language: Some("markdown"),
        },
        Some("txt") => ViewKind::Markdown { language: None },
        Some("csv" | "tsv") if enabled("csv") => ViewKind::Csv,
        Some("json") if enabled("json") => ViewKind::Json,
        _ => {
            let language = if enabled("codefiles") {
                highlight::language_for_path(path)
            } else {
                None
            };
            ViewKind::Text { language }
        }
    }
}

/// The modes a view offers, in tab-bar order. A single mode means no switcher.
pub fn modes(kind: ViewKind) -> &'static [&'static str] {
    match kind {
        ViewKind::Markdown { .. } => &["rich", "plain"],
        ViewKind::Text { .. } => &["plain"],
        ViewKind::Csv => &["table", "text"],
        ViewKind::Json => &["text", "tree", "table"],
    }
}

/// The label a mode button shows.
pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "rich" => "Rich",
        "plain" => "Plain",
        "table" => "Table",
        "text" => "Text",
        "tree" => "Tree",
        _ => "",
    }
}

/// Turns a persisted mode into one the view offers. Markdown and plain text
/// fall back to `plain`. CSV and JSON return `None` for an unknown or missing
/// mode, so the view keeps its own default (JSON opens as Tree only when it
/// parses, CSV as Table only when it parses).
pub fn normalize_mode(kind: ViewKind, stored: &str) -> Option<&'static str> {
    if let Some(mode) = modes(kind).iter().copied().find(|mode| *mode == stored) {
        return Some(mode);
    }
    match kind {
        ViewKind::Markdown { .. } | ViewKind::Text { .. } => Some("plain"),
        ViewKind::Csv | ViewKind::Json => None,
    }
}

/// The mode a file opens in when nothing was saved for it. Markdown and `.txt`
/// open rendered (`rich`); other text opens in the editor. `None` leaves CSV
/// and JSON to their own default.
pub fn default_mode(kind: ViewKind) -> Option<&'static str> {
    match kind {
        ViewKind::Markdown { .. } => Some("rich"),
        ViewKind::Text { .. } => Some("plain"),
        ViewKind::Csv | ViewKind::Json => None,
    }
}

/// The mode after `current` when Cmd-E cycles the view, or `None` when the view
/// has only one mode.
pub fn next_mode(kind: ViewKind, current: &str) -> Option<&'static str> {
    let modes = modes(kind);
    if modes.len() < 2 {
        return None;
    }
    let index = modes.iter().position(|mode| *mode == current).unwrap_or(0);
    Some(modes[(index + 1) % modes.len()])
}

/// The name the status bar shows for a highlighted language id (`highlight`
/// ids). Files with no language read as plain text.
pub fn language_label(language: Option<&str>) -> &'static str {
    match language {
        Some("javascript") => "JavaScript",
        Some("typescript") => "TypeScript",
        Some("jsx") => "JSX",
        Some("tsx") => "TSX",
        Some("json") => "JSON",
        Some("python") => "Python",
        Some("html") => "HTML",
        Some("css") => "CSS",
        Some("yaml") => "YAML",
        Some("xml") => "XML",
        Some("sql") => "SQL",
        Some("rust") => "Rust",
        Some("go") => "Go",
        Some("java") => "Java",
        Some("cpp") => "C++",
        Some("c") => "C",
        Some("php") => "PHP",
        Some("markdown") => "Markdown",
        Some("shell") => "Shell",
        Some("toml") => "TOML",
        Some("dockerfile") => "Dockerfile",
        Some("ruby") => "Ruby",
        Some("swift") => "Swift",
        Some("kotlin") => "Kotlin",
        Some("csharp") => "C#",
        Some("properties") => "Properties",
        Some("diff") => "Diff",
        _ => "Plain text",
    }
}

/// Whether the mode shows an editor buffer, which is what Find and Replace work on.
pub fn shows_text(kind: ViewKind, mode: &str) -> bool {
    match kind {
        ViewKind::Markdown { .. } => mode == "plain",
        ViewKind::Text { .. } => true,
        ViewKind::Csv | ViewKind::Json => mode == "text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn markdown_and_txt_keep_rich_and_plain() {
        let md = kind_for_path("/notes/Doc.MD", all);
        assert_eq!(
            md,
            ViewKind::Markdown {
                language: Some("markdown")
            }
        );
        assert_eq!(modes(md), &["rich", "plain"]);
        assert_eq!(
            kind_for_path("/notes/readme.txt", all),
            ViewKind::Markdown { language: None }
        );
    }

    #[test]
    fn csv_and_json_follow_their_extension_switches() {
        assert_eq!(kind_for_path("/d/data.csv", all), ViewKind::Csv);
        assert_eq!(kind_for_path("/d/data.TSV", all), ViewKind::Csv);
        assert_eq!(kind_for_path("/d/data.json", all), ViewKind::Json);

        let csv_off = |id: &str| id != "csv";
        assert_eq!(
            kind_for_path("/d/data.csv", csv_off),
            ViewKind::Text { language: None }
        );
        let json_off = |id: &str| id != "json";
        assert_eq!(
            kind_for_path("/d/data.json", json_off),
            ViewKind::Text {
                language: Some("json")
            }
        );
    }

    #[test]
    fn code_files_colour_only_with_codefiles_on() {
        assert_eq!(
            kind_for_path("/p/config.yaml", all),
            ViewKind::Text {
                language: Some("yaml")
            }
        );
        assert_eq!(
            kind_for_path("/p/main.rs", none),
            ViewKind::Text { language: None }
        );
        assert_eq!(
            kind_for_path("/p/app.TS", all),
            ViewKind::Text {
                language: Some("typescript")
            }
        );
    }

    #[test]
    fn unknown_and_extensionless_files_are_plain_text() {
        assert_eq!(
            kind_for_path("/p/Makefile", all),
            ViewKind::Text { language: None }
        );
        assert_eq!(
            kind_for_path("/p/.env", all),
            ViewKind::Text {
                language: Some("properties")
            }
        );
        assert_eq!(
            kind_for_path("/p/archive.bin", all),
            ViewKind::Text { language: None }
        );
    }

    #[test]
    fn mode_cycling_follows_the_view() {
        let md = ViewKind::Markdown { language: None };
        assert_eq!(next_mode(md, "rich"), Some("plain"));
        assert_eq!(next_mode(md, "plain"), Some("rich"));

        assert_eq!(next_mode(ViewKind::Json, "text"), Some("tree"));
        assert_eq!(next_mode(ViewKind::Json, "tree"), Some("table"));
        assert_eq!(next_mode(ViewKind::Json, "table"), Some("text"));

        assert_eq!(next_mode(ViewKind::Csv, "table"), Some("text"));
        assert_eq!(next_mode(ViewKind::Csv, "text"), Some("table"));

        assert_eq!(next_mode(ViewKind::Text { language: None }, "plain"), None);
    }

    #[test]
    fn persisted_modes_are_normalised_per_view() {
        let md = ViewKind::Markdown { language: None };
        assert_eq!(normalize_mode(md, "rich"), Some("rich"));
        assert_eq!(normalize_mode(md, "plain"), Some("plain"));
        assert_eq!(normalize_mode(md, "tree"), Some("plain"));
        assert_eq!(normalize_mode(md, ""), Some("plain"));

        assert_eq!(normalize_mode(ViewKind::Json, "tree"), Some("tree"));
        assert_eq!(normalize_mode(ViewKind::Json, "rich"), None);
        assert_eq!(normalize_mode(ViewKind::Json, ""), None);

        assert_eq!(normalize_mode(ViewKind::Csv, "text"), Some("text"));
        assert_eq!(normalize_mode(ViewKind::Csv, "tree"), None);

        let text = ViewKind::Text {
            language: Some("yaml"),
        };
        assert_eq!(normalize_mode(text, "rich"), Some("plain"));
    }

    #[test]
    fn find_only_targets_visible_text_buffers() {
        assert!(shows_text(ViewKind::Markdown { language: None }, "plain"));
        assert!(!shows_text(ViewKind::Markdown { language: None }, "rich"));
        assert!(shows_text(ViewKind::Json, "text"));
        assert!(!shows_text(ViewKind::Json, "tree"));
        assert!(!shows_text(ViewKind::Json, "table"));
        assert!(shows_text(ViewKind::Csv, "text"));
        assert!(!shows_text(ViewKind::Csv, "table"));
        assert!(shows_text(ViewKind::Text { language: None }, "plain"));
    }

    #[test]
    fn status_names_the_language_of_a_file() {
        assert_eq!(language_label(Some("yaml")), "YAML");
        assert_eq!(language_label(Some("rust")), "Rust");
        assert_eq!(language_label(Some("cpp")), "C++");
        assert_eq!(language_label(None), "Plain text");
        assert_eq!(language_label(Some("unknown-id")), "Plain text");
    }

    #[test]
    fn labels_match_the_web_build() {
        assert_eq!(mode_label("rich"), "Rich");
        assert_eq!(mode_label("tree"), "Tree");
        assert_eq!(mode_label("table"), "Table");
        assert_eq!(mode_label("text"), "Text");
    }
}
