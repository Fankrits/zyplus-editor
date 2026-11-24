//! Cheap per-tab text facts that would otherwise be recomputed from the full text on every
//! render or keystroke: a fingerprint to tell whether the text matches what is on disk, and
//! the word count shown in the status bar.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Longest text whose words are counted on every edit. Longer text is counted after typing
/// pauses, so a large file does not rescan itself per keystroke.
pub const WORDS_SYNC_LIMIT: usize = 128 * 1024;

pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The length and a 64-bit hash of a text. Replaces keeping a second copy of the saved text
/// just to ask "has it changed since the last save".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    len: usize,
    hash: u64,
}

impl Fingerprint {
    pub fn of(text: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        Self {
            len: text.len(),
            hash: hasher.finish(),
        }
    }

    /// Whether `text` is the text this was taken from. Texts of a different length are
    /// rejected without hashing them, which is almost every keystroke.
    pub fn matches(&self, text: &str) -> bool {
        self.len == text.len() && Self::of(text) == *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_matches_only_its_own_text() {
        let saved = Fingerprint::of("hello world");
        assert!(saved.matches("hello world"));
        assert!(!saved.matches("hello worlds"));
        assert!(!saved.matches("hello w0rld"), "same length, different text");
        assert!(!saved.matches(""));
        assert!(Fingerprint::of("").matches(""));
    }

    #[test]
    fn multibyte_text_fingerprints_by_bytes() {
        let saved = Fingerprint::of("สวัสดี café");
        assert!(saved.matches("สวัสดี café"));
        assert!(!saved.matches("สวัสดี cafe"));
    }

    #[test]
    fn words_are_counted_across_any_whitespace() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("  one\ttwo\nthree  four "), 4);
        assert_eq!(count_words("สวัสดี ครับ"), 2);
    }
}
