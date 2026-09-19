//! Command history with optional on-disk persistence.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Scrollback of previously submitted commands.
///
/// Navigation keeps the half-typed line around, so Up/Down/Up returns the
/// operator to exactly what they were writing.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<String>,
    /// `None` means "editing a fresh line"; otherwise an index into `entries`.
    position: Option<usize>,
    draft: String,
    limit: usize,
    path: Option<PathBuf>,
}

impl History {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            position: None,
            draft: String::new(),
            limit: limit.max(1),
            path: None,
        }
    }

    /// Load history from `path`, remembering it for [`Self::save`].
    ///
    /// A missing file is not an error — that is simply the first run.
    pub fn load(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref().to_path_buf();
        match fs::read_to_string(&path) {
            Ok(contents) => {
                self.entries = contents
                    .lines()
                    .map(str::trim_end)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                self.trim();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.path = Some(path);
        Ok(())
    }

    /// Write history back to the file passed to [`Self::load`]. No-op if the
    /// history is memory-only.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.entries.join("\n"))
    }

    pub fn push(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        self.reset();
        if entry.trim().is_empty() {
            return;
        }
        // Consecutive duplicates just move to the end of the list.
        if self.entries.last() == Some(&entry) {
            return;
        }
        self.entries.push(entry);
        self.trim();
    }

    /// Step towards older entries. Pass the line currently being edited so it
    /// can be restored on the way back down.
    pub fn older(&mut self, current: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.position {
            None => {
                self.draft.clear();
                self.draft.push_str(current);
                self.entries.len() - 1
            }
            Some(0) => 0,
            Some(index) => index - 1,
        };
        self.position = Some(next);
        self.entries.get(next).map(String::as_str)
    }

    /// Step towards newer entries, ending on the preserved draft.
    pub fn newer(&mut self) -> Option<&str> {
        match self.position {
            None => None,
            Some(index) if index + 1 < self.entries.len() => {
                self.position = Some(index + 1);
                self.entries.get(index + 1).map(String::as_str)
            }
            Some(_) => {
                self.position = None;
                Some(&self.draft)
            }
        }
    }

    /// Forget where we were browsing (called on any edit).
    pub const fn reset(&mut self) {
        self.position = None;
    }

    #[must_use]
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    fn trim(&mut self) {
        if self.entries.len() > self.limit {
            let excess = self.entries.len() - self.limit;
            self.entries.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::History;

    #[test]
    fn browses_and_restores_the_draft() {
        let mut history = History::new(16);
        history.push("stop");
        history.push("say hello");

        assert_eq!(history.older("tim"), Some("say hello"));
        assert_eq!(history.older("tim"), Some("stop"));
        assert_eq!(history.older("tim"), Some("stop"));
        assert_eq!(history.newer(), Some("say hello"));
        assert_eq!(history.newer(), Some("tim"));
        assert_eq!(history.newer(), None);
    }

    #[test]
    fn skips_blank_and_repeated_entries() {
        let mut history = History::new(16);
        history.push("stop");
        history.push("stop");
        history.push("   ");
        assert_eq!(history.entries(), ["stop"]);
    }

    #[test]
    fn honours_the_limit() {
        let mut history = History::new(2);
        history.push("a");
        history.push("b");
        history.push("c");
        assert_eq!(history.entries(), ["b", "c"]);
    }
}
