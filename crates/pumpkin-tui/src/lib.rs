//! The server console UI: log pane, status header, player sidebar, and a
//! command line with completion, history, and filtering.
//!
//! The crate deliberately knows nothing about the server itself. `pumpkin`
//! feeds it log records and status through a [`ConsoleHandle`], answers
//! completion queries with a [`Completer`] over the command dispatcher, and
//! reads back operator input; that wiring lives in `pumpkin::console`.

pub mod app;
pub mod backend;
pub mod channel;
pub mod completion;
pub mod history;
pub mod input;
pub mod theme;
pub(crate) mod ui;
mod wrap;

pub use app::{Console, ConsoleConfig};
pub use backend::{
    ConsoleBackend, ConsoleEvent, ConsoleInput, Level, LogRecord, PlayerInfo, ServerStatus,
};
pub use channel::{ChannelBackend, ConsoleHandle, console_channel};
pub use completion::{Completer, Completion, CompletionKind, CompletionRequest, Completions};
pub use history::History;
pub use input::LineEditor;
pub use theme::Theme;

/// Hand the terminal back: leave the alternate screen and disable raw mode.
///
/// [`Console::run`] already does this on the way out, including on panic. Call
/// it directly only from a panic or crash handler, where the process is about
/// to print to a terminal the console may still own. It is safe to call when no
/// console is running.
pub fn restore_terminal() {
    ratatui::restore();
}
