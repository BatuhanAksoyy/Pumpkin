//! Command completion: the trait the console asks, plus a batteries-included
//! tree implementation for hosts that do not already have a dispatcher able to
//! answer completion queries.

use std::sync::Arc;

/// What the operator has typed so far.
#[derive(Clone, Copy, Debug)]
pub struct CompletionRequest<'a> {
    /// The full input line (no leading `/` is required, but one is tolerated).
    pub line: &'a str,
    /// Byte offset of the caret within `line`.
    pub cursor: usize,
}

impl CompletionRequest<'_> {
    /// The line up to the caret; completion never looks past it.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.line[..self.cursor.min(self.line.len())]
    }

    /// Byte offset where the word under the caret starts.
    #[must_use]
    pub fn word_start(&self) -> usize {
        let prefix = self.prefix();
        prefix
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(idx, c)| idx + c.len_utf8())
    }

    /// The (possibly empty) word under the caret.
    #[must_use]
    pub fn word(&self) -> &str {
        &self.prefix()[self.word_start()..]
    }
}

/// What a candidate represents, used purely for colouring the popup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompletionKind {
    /// A root command name.
    Command,
    /// A fixed sub-command word.
    Literal,
    /// A concrete value for an argument.
    Argument,
    /// An online player (or any entity selector target).
    Player,
    /// A `<placeholder>` describing the expected argument. Shown as guidance;
    /// the console never inserts it into the line.
    Placeholder,
}

/// One candidate.
#[derive(Clone, Debug)]
pub struct Completion {
    pub value: String,
    pub detail: Option<String>,
    pub kind: CompletionKind,
}

impl Completion {
    #[must_use]
    pub fn new(value: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            value: value.into(),
            detail: None,
            kind,
        }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Whether accepting this candidate should rewrite the line.
    #[must_use]
    pub const fn insertable(&self) -> bool {
        !matches!(self.kind, CompletionKind::Placeholder)
    }
}

/// A completion result: the candidates, plus where in the line they go.
#[derive(Clone, Debug, Default)]
pub struct Completions {
    /// Byte offset in the line that a chosen `value` replaces from.
    pub start: usize,
    pub items: Vec<Completion>,
}

impl Completions {
    #[must_use]
    pub const fn new(start: usize, items: Vec<Completion>) -> Self {
        Self { start, items }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Longest prefix shared by every insertable candidate — what Tab should
    /// fill in when the choice is still ambiguous.
    #[must_use]
    pub fn common_prefix(&self) -> Option<String> {
        let mut insertable = self.items.iter().filter(|item| item.insertable());
        let mut prefix: String = insertable.next()?.value.clone();
        for item in insertable {
            let shared = prefix
                .char_indices()
                .zip(item.value.chars())
                .take_while(|((_, a), b)| a == b)
                .last()
                .map_or(0, |((idx, a), _)| idx + a.len_utf8());
            prefix.truncate(shared);
            if prefix.is_empty() {
                return None;
            }
        }
        (!prefix.is_empty()).then_some(prefix)
    }
}

/// Answers completion queries for the console.
///
/// Implement this over your own command dispatcher; it is called from the UI
/// thread on every keystroke, so keep it cheap and never block on I/O.
pub trait Completer: Send + Sync {
    fn complete(&self, request: CompletionRequest<'_>) -> Completions;
}

/// A completer that never suggests anything.
pub struct NoCompleter;

impl Completer for NoCompleter {
    fn complete(&self, _request: CompletionRequest<'_>) -> Completions {
        Completions::default()
    }
}

impl<F> Completer for F
where
    F: Fn(CompletionRequest<'_>) -> Completions + Send + Sync,
{
    fn complete(&self, request: CompletionRequest<'_>) -> Completions {
        self(request)
    }
}

type SuggestFn = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

#[derive(Clone)]
enum Suggest {
    None,
    Static(Vec<String>),
    Dynamic(SuggestFn),
}

impl Suggest {
    fn values(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Static(values) => values.clone(),
            Self::Dynamic(source) => source(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Literal,
    Argument,
}

/// One node of a [`CommandTree`]: either a fixed word or an argument slot.
#[derive(Clone)]
pub struct Node {
    kind: NodeKind,
    name: String,
    detail: Option<String>,
    suggest: Suggest,
    children: Vec<Self>,
}

impl Node {
    /// A fixed word, e.g. `gamemode` or `add`.
    #[must_use]
    pub fn literal(name: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Literal,
            name: name.into(),
            detail: None,
            suggest: Suggest::None,
            children: Vec::new(),
        }
    }

    /// An argument slot. `placeholder` is what the UI shows when it has no
    /// concrete values to offer, conventionally `<player>`.
    #[must_use]
    pub fn argument(placeholder: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Argument,
            name: placeholder.into(),
            detail: None,
            suggest: Suggest::None,
            children: Vec::new(),
        }
    }

    /// Help text shown next to the candidate.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Fixed set of values for an argument node.
    #[must_use]
    pub fn suggest<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.suggest = Suggest::Static(values.into_iter().map(Into::into).collect());
        self
    }

    /// Values computed at completion time — online players, loaded worlds, and
    /// anything else that changes while the server runs.
    #[must_use]
    pub fn suggest_with<F>(mut self, source: F) -> Self
    where
        F: Fn() -> Vec<String> + Send + Sync + 'static,
    {
        self.suggest = Suggest::Dynamic(Arc::new(source));
        self
    }

    /// Attach a follow-up node.
    #[must_use]
    pub fn then(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    fn matches(&self, token: &str) -> bool {
        match self.kind {
            NodeKind::Literal => self.name == token,
            NodeKind::Argument => true,
        }
    }

    fn candidates(&self, depth: usize) -> Vec<Completion> {
        match self.kind {
            NodeKind::Literal => {
                let kind = if depth == 0 {
                    CompletionKind::Command
                } else {
                    CompletionKind::Literal
                };
                let mut completion = Completion::new(self.name.clone(), kind);
                completion.detail.clone_from(&self.detail);
                vec![completion]
            }
            NodeKind::Argument => {
                let values = self.suggest.values();
                if values.is_empty() {
                    let mut completion =
                        Completion::new(self.name.clone(), CompletionKind::Placeholder);
                    completion.detail.clone_from(&self.detail);
                    vec![completion]
                } else {
                    values
                        .into_iter()
                        .map(|value| {
                            let mut completion = Completion::new(value, CompletionKind::Argument);
                            completion.detail.clone_from(&self.detail);
                            completion
                        })
                        .collect()
                }
            }
        }
    }
}

/// A static command tree — enough to drive completion for a demo, a plugin's
/// own sub-commands, or a server whose dispatcher cannot be queried directly.
#[derive(Clone, Default)]
pub struct CommandTree {
    roots: Vec<Node>,
}

impl CommandTree {
    #[must_use]
    pub const fn new() -> Self {
        Self { roots: Vec::new() }
    }

    /// Register a root command.
    #[must_use]
    pub fn with(mut self, node: Node) -> Self {
        self.roots.push(node);
        self
    }
}

impl Completer for CommandTree {
    fn complete(&self, request: CompletionRequest<'_>) -> Completions {
        let prefix = request.prefix();
        // A leading slash is optional on a server console; ignore it while
        // walking, but keep byte offsets pointing into the real line.
        let skip = usize::from(prefix.starts_with('/'));
        let walked = &prefix[skip..];

        let word_start = request.word_start().max(skip);
        let word = &prefix[word_start..];

        let mut level: &[Node] = &self.roots;
        let mut depth = 0usize;

        // Every whitespace-delimited token before the caret's word is settled
        // input; walk it to find which node we are completing under.
        for token in walked[..walked.len() - word.len()].split_whitespace() {
            let Some(node) = level
                .iter()
                .find(|node| node.kind == NodeKind::Literal && node.matches(token))
                .or_else(|| level.iter().find(|node| node.matches(token)))
            else {
                return Completions::new(word_start, Vec::new());
            };
            level = &node.children;
            depth += 1;
        }

        let items = level
            .iter()
            .flat_map(|node| node.candidates(depth))
            .filter(|item| item.kind == CompletionKind::Placeholder || item.value.starts_with(word))
            .collect();

        Completions::new(word_start, items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> CommandTree {
        CommandTree::new()
            .with(
                Node::literal("gamemode")
                    .detail("Change a player's game mode")
                    .then(
                        Node::argument("<mode>")
                            .suggest(["survival", "creative", "spectator"])
                            .then(Node::argument("<player>").suggest(["Notch", "Steve"])),
                    ),
            )
            .with(Node::literal("gamerule").then(Node::argument("<rule>")))
            .with(Node::literal("stop").detail("Stop the server"))
    }

    fn complete(line: &str) -> Vec<String> {
        tree()
            .complete(CompletionRequest {
                line,
                cursor: line.len(),
            })
            .items
            .into_iter()
            .map(|item| item.value)
            .collect()
    }

    #[test]
    fn completes_root_commands_by_prefix() {
        assert_eq!(complete("game"), vec!["gamemode", "gamerule"]);
        assert_eq!(complete(""), vec!["gamemode", "gamerule", "stop"]);
    }

    #[test]
    fn tolerates_leading_slash() {
        assert_eq!(complete("/sto"), vec!["stop"]);
    }

    #[test]
    fn walks_into_arguments() {
        assert_eq!(
            complete("gamemode "),
            vec!["survival", "creative", "spectator"]
        );
        assert_eq!(complete("gamemode c"), vec!["creative"]);
        assert_eq!(complete("gamemode creative "), vec!["Notch", "Steve"]);
    }

    #[test]
    fn offers_placeholder_when_no_values_are_known() {
        let items = tree().complete(CompletionRequest {
            line: "gamerule ",
            cursor: 9,
        });
        assert_eq!(items.items.len(), 1);
        assert_eq!(items.items[0].kind, CompletionKind::Placeholder);
        assert!(!items.items[0].insertable());
    }

    #[test]
    fn unknown_command_yields_nothing() {
        assert!(complete("nonsense ").is_empty());
    }

    #[test]
    fn start_offset_points_at_the_word() {
        let line = "gamemode cre";
        let completions = tree().complete(CompletionRequest {
            line,
            cursor: line.len(),
        });
        assert_eq!(completions.start, 9);
        assert_eq!(completions.common_prefix().as_deref(), Some("creative"));
    }
}
