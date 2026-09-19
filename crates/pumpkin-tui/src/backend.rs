//! The data that flows between a host server and the console UI.
//!
//! Everything in this module is plain data plus one small trait, so a host can
//! feed the UI without depending on `ratatui`, `tokio`, or anything else the
//! console happens to use internally.

use std::time::Duration;

/// Severity of a log record, mirroring the levels `tracing` and `log` use.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    /// Every level, quietest first.
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];

    /// Fixed-width label used in the log gutter.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO ",
            Self::Warn => "WARN ",
            Self::Error => "ERROR",
        }
    }
}

/// A single line of server output.
#[derive(Clone, Debug)]
pub struct LogRecord {
    pub level: Level,
    /// Pre-formatted timestamp. The host owns the format so the UI never has to
    /// pull in a date library; pass an empty string to hide it.
    pub time: String,
    /// Emitting module/subsystem, e.g. `pumpkin::net`.
    pub target: Option<String>,
    pub message: String,
}

impl LogRecord {
    #[must_use]
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        Self {
            level,
            time: String::new(),
            target: None,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn with_time(mut self, time: impl Into<String>) -> Self {
        self.time = time.into();
        self
    }

    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Text the search/filter box matches against.
    #[must_use]
    pub fn searchable(&self) -> String {
        self.target.as_ref().map_or_else(
            || self.message.clone(),
            |target| format!("{target} {}", self.message),
        )
    }
}

/// Counters shown in the header bar. All fields are optional in spirit: leave a
/// value at its default and the header simply renders it as unknown.
#[derive(Clone, Debug)]
pub struct ServerStatus {
    pub brand: String,
    pub version: String,
    pub tps: f64,
    pub mspt: f64,
    pub players_online: u32,
    pub players_max: u32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub chunks_loaded: u64,
    pub uptime: Duration,
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self {
            brand: "Pumpkin".to_owned(),
            version: String::new(),
            tps: 20.0,
            mspt: 0.0,
            players_online: 0,
            players_max: 0,
            memory_used_mb: 0,
            memory_total_mb: 0,
            chunks_loaded: 0,
            uptime: Duration::ZERO,
        }
    }
}

/// One entry in the player sidebar.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub name: String,
    pub world: String,
    pub ping_ms: u32,
}

/// Something the server tells the console.
#[derive(Clone, Debug)]
pub enum ConsoleEvent {
    Log(LogRecord),
    Status(ServerStatus),
    Players(Vec<PlayerInfo>),
    /// Server is going away; the console should tear down and return.
    Close,
}

/// Something the console tells the server.
#[derive(Clone, Debug)]
pub enum ConsoleInput {
    /// A command line the operator submitted, without any leading `/`.
    Command(String),
    /// The operator asked to leave the console (Ctrl+C, Ctrl+D, confirmed quit).
    Quit,
}

/// The console's view of a running server.
///
/// [`crate::channel::console_channel`] provides the implementation most hosts
/// want; implement this directly only if you already have your own event loop
/// to pull from.
pub trait ConsoleBackend: Send {
    /// Non-blocking: return the next pending event, or `None` if there is none.
    fn poll_event(&mut self) -> Option<ConsoleEvent>;

    /// Deliver operator input to the server. Never blocks for long.
    fn send(&mut self, input: ConsoleInput);
}
