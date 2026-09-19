//! A small readline-style line editor: the buffer, the caret, and the motions
//! an operator expects from a server console.

use unicode_width::UnicodeWidthStr;

/// Editable input line. Offsets are byte indices that always sit on a `char`
/// boundary.
#[derive(Clone, Debug, Default)]
pub struct LineEditor {
    buffer: String,
    cursor: usize,
}

impl LineEditor {
    #[must_use]
    pub fn text(&self) -> &str {
        &self.buffer
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Display columns between the start of the line and the caret, so the
    /// renderer can place the terminal cursor over wide characters correctly.
    #[must_use]
    pub fn cursor_column(&self) -> usize {
        self.buffer[..self.cursor].width()
    }

    pub fn set(&mut self, text: impl Into<String>) {
        self.buffer = text.into();
        self.cursor = self.buffer.len();
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Take the line, leaving the editor empty.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.buffer)
    }

    pub fn insert(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn insert_str(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Replace `range` with `text` and park the caret after it — the shape
    /// every completion accept needs.
    pub fn replace_range(&mut self, start: usize, end: usize, text: &str) {
        let start = start.min(self.buffer.len());
        let end = end.clamp(start, self.buffer.len());
        self.buffer.replace_range(start..end, text);
        self.cursor = start + text.len();
    }

    pub fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary(self.cursor) {
            self.buffer.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if let Some(next) = self.next_boundary(self.cursor) {
            self.buffer.replace_range(self.cursor..next, "");
        }
    }

    pub fn move_left(&mut self) {
        if let Some(prev) = self.prev_boundary(self.cursor) {
            self.cursor = prev;
        }
    }

    pub fn move_right(&mut self) {
        if let Some(next) = self.next_boundary(self.cursor) {
            self.cursor = next;
        }
    }

    pub const fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub const fn move_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    pub fn move_word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn move_word_right(&mut self) {
        self.cursor = self.word_end();
    }

    /// Ctrl+W: delete the word before the caret.
    pub fn delete_word_left(&mut self) {
        let start = self.word_start();
        self.buffer.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Alt+D: delete the word after the caret.
    pub fn delete_word_right(&mut self) {
        let end = self.word_end();
        self.buffer.replace_range(self.cursor..end, "");
    }

    /// Ctrl+U: drop everything before the caret.
    pub fn kill_to_start(&mut self) {
        self.buffer.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// Ctrl+K: drop everything after the caret.
    pub fn kill_to_end(&mut self) {
        self.buffer.truncate(self.cursor);
    }

    /// Whether the caret sits at the very end — the condition for showing an
    /// inline ghost hint.
    #[must_use]
    pub const fn at_end(&self) -> bool {
        self.cursor == self.buffer.len()
    }

    fn prev_boundary(&self, from: usize) -> Option<usize> {
        self.buffer[..from]
            .char_indices()
            .next_back()
            .map(|(idx, _)| idx)
    }

    fn next_boundary(&self, from: usize) -> Option<usize> {
        self.buffer[from..]
            .chars()
            .next()
            .map(|c| from + c.len_utf8())
    }

    fn word_start(&self) -> usize {
        let before = &self.buffer[..self.cursor];
        let trimmed = before.trim_end();
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(idx, c)| idx + c.len_utf8())
    }

    fn word_end(&self) -> usize {
        let after = &self.buffer[self.cursor..];
        let leading = after.len() - after.trim_start().len();
        let rest = &after[leading..];
        let word = rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map_or(rest.len(), |(idx, _)| idx);
        self.cursor + leading + word
    }
}

#[cfg(test)]
mod tests {
    use super::LineEditor;

    #[test]
    fn edits_and_motions() {
        let mut editor = LineEditor::default();
        editor.set("gamemode creative");
        assert_eq!(editor.cursor(), 17);

        editor.move_word_left();
        assert_eq!(editor.cursor(), 9);

        editor.delete_word_left();
        assert_eq!(editor.text(), "creative");

        editor.move_home();
        editor.insert_str("say ");
        assert_eq!(editor.text(), "say creative");
        assert_eq!(editor.cursor(), 4);

        editor.kill_to_end();
        assert_eq!(editor.text(), "say ");
    }

    #[test]
    fn handles_multibyte_text() {
        let mut editor = LineEditor::default();
        editor.set("say Grüße");
        editor.backspace();
        assert_eq!(editor.text(), "say Grüß");
        editor.move_left();
        editor.delete();
        assert_eq!(editor.text(), "say Grü");
        assert_eq!(editor.cursor_column(), 7);
    }

    #[test]
    fn replaces_completion_range() {
        let mut editor = LineEditor::default();
        editor.set("gamemode cre");
        editor.replace_range(9, 12, "creative");
        assert_eq!(editor.text(), "gamemode creative");
        assert!(editor.at_end());
    }
}
