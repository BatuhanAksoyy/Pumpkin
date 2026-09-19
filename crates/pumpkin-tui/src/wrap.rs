//! Width-aware word wrapping.
//!
//! The log pane wraps by hand instead of leaning on `Paragraph`, because
//! scrollback has to be counted in *rendered* lines and the search highlight
//! needs to survive the wrap.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Wrap `text` to `width` display columns, breaking on whitespace where
/// possible and splitting words that are too long to ever fit.
///
/// Returns at least one (possibly empty) line.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in split_keeping_spaces(text) {
        let word_width = word.width();

        // A space that lands exactly on the edge is dropped rather than
        // pushed onto the next line.
        if word.chars().all(char::is_whitespace) && current_width + word_width > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
            continue;
        }

        if current_width + word_width <= width {
            current.push_str(word);
            current_width += word_width;
            continue;
        }

        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }

        if word_width <= width {
            current.push_str(word);
            current_width = word_width;
            continue;
        }

        // Word longer than the pane: hard-split it.
        for c in word.chars() {
            let char_width = c.width().unwrap_or(0);
            if current_width + char_width > width {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(c);
            current_width += char_width;
        }
    }

    lines.push(current);
    lines
}

/// Split into alternating runs of whitespace and non-whitespace, so wrapping
/// can keep interior spacing intact.
fn split_keeping_spaces(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut previous_kind = None;

    for (idx, c) in text.char_indices() {
        let is_space = c.is_whitespace();
        match previous_kind {
            Some(previous) if previous != is_space => {
                parts.push(&text[start..idx]);
                start = idx;
            }
            _ => {}
        }
        previous_kind = Some(is_space);
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

/// Byte ranges of every case-insensitive occurrence of `needle` in `haystack`.
#[must_use]
pub fn find_matches(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let hay = haystack.to_lowercase();
    let pin = needle.to_lowercase();
    // Lowercasing can change byte lengths (e.g. 'İ'), which would invalidate
    // offsets; fall back to a case-sensitive scan in that rare case.
    let (hay, pin) = if hay.len() == haystack.len() && pin.len() == needle.len() {
        (hay, pin)
    } else {
        (haystack.to_owned(), needle.to_owned())
    };

    let mut matches = Vec::new();
    let mut from = 0;
    while let Some(found) = hay[from..].find(&pin) {
        let start = from + found;
        let end = start + pin.len();
        matches.push((start, end));
        from = end;
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::{find_matches, wrap};

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(wrap("the quick brown fox", 10), ["the quick ", "brown fox"]);
    }

    #[test]
    fn splits_words_that_cannot_fit() {
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn empty_text_is_one_empty_line() {
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("hi", 0), [""]);
    }

    #[test]
    fn accounts_for_wide_characters() {
        // Each CJK glyph is two columns wide.
        assert_eq!(wrap("日本語テスト", 4), ["日本", "語テ", "スト"]);
    }

    #[test]
    fn finds_matches_case_insensitively() {
        assert_eq!(find_matches("Player Steve joined", "steve"), [(7, 12)]);
        assert_eq!(find_matches("aaa", "a"), [(0, 1), (1, 2), (2, 3)]);
        assert!(find_matches("abc", "").is_empty());
    }
}
