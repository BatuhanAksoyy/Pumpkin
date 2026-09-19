//! A ready-made [`ConsoleBackend`] built on channels.
//!
//! The host keeps a [`ConsoleHandle`] (cheap to clone, `Send + Sync`, safe to
//! call from a logging layer or any worker thread) and hands the paired
//! [`ChannelBackend`] to [`crate::Console`].

use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::backend::{
    ConsoleBackend, ConsoleEvent, ConsoleInput, Level, LogRecord, PlayerInfo, ServerStatus,
};

/// Create a connected handle/backend pair.
///
/// `capacity` bounds the queue of pending events; when it fills up,
/// [`ConsoleHandle::log`] drops the record rather than blocking the server on a
/// slow terminal.
#[must_use]
pub fn console_channel(capacity: usize) -> (ConsoleHandle, ChannelBackend) {
    let (event_tx, event_rx) = sync_channel(capacity);
    let (input_tx, input_rx) = std::sync::mpsc::channel();
    (
        ConsoleHandle {
            events: event_tx,
            inputs: Arc::new(Mutex::new(input_rx)),
            dropped: Arc::new(Mutex::new(0)),
        },
        ChannelBackend {
            events: event_rx,
            inputs: input_tx,
        },
    )
}

/// The server-side end of the console.
#[derive(Clone)]
pub struct ConsoleHandle {
    events: SyncSender<ConsoleEvent>,
    inputs: Arc<Mutex<Receiver<ConsoleInput>>>,
    dropped: Arc<Mutex<u64>>,
}

// The `bool` returns say whether the console is still listening; a host that
// does not care is free to ignore them.
#[allow(clippy::must_use_candidate)]
impl ConsoleHandle {
    /// Push a log record to the console. Returns `false` if it was dropped
    /// because the console is backed up or gone.
    pub fn log(&self, record: LogRecord) -> bool {
        self.emit(ConsoleEvent::Log(record))
    }

    /// Convenience wrapper for a plain message.
    pub fn log_message(&self, level: Level, message: impl Into<String>) -> bool {
        self.log(LogRecord::new(level, message))
    }

    /// Replace the counters in the header bar.
    pub fn set_status(&self, status: ServerStatus) -> bool {
        self.emit(ConsoleEvent::Status(status))
    }

    /// Replace the player sidebar contents.
    pub fn set_players(&self, players: Vec<PlayerInfo>) -> bool {
        self.emit(ConsoleEvent::Players(players))
    }

    /// Ask the console to shut down and return from [`crate::Console::run`].
    pub fn close(&self) -> bool {
        self.emit(ConsoleEvent::Close)
    }

    /// How many events were dropped because the queue was full. Worth logging
    /// once at shutdown if you care about lost output.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.dropped.lock().map_or(0, |count| *count)
    }

    /// Block until the operator submits something, or the console goes away.
    #[must_use]
    pub fn next_input(&self) -> Option<ConsoleInput> {
        let inputs = self.inputs.lock().ok()?;
        inputs.recv().ok()
    }

    /// Like [`Self::next_input`], but gives up after `timeout` so a caller can
    /// check its own shutdown flag between polls.
    #[must_use]
    pub fn next_input_timeout(&self, timeout: Duration) -> Option<ConsoleInput> {
        let inputs = self.inputs.lock().ok()?;
        inputs.recv_timeout(timeout).ok()
    }

    /// Non-blocking variant, for hosts that drive their own tick loop.
    #[must_use]
    pub fn try_next_input(&self) -> Option<ConsoleInput> {
        let inputs = self.inputs.lock().ok()?;
        inputs.try_recv().ok()
    }

    fn emit(&self, event: ConsoleEvent) -> bool {
        match self.events.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                if let Ok(mut dropped) = self.dropped.lock() {
                    *dropped += 1;
                }
                false
            }
        }
    }
}

/// The console-side end, handed to [`crate::Console::new`].
pub struct ChannelBackend {
    events: Receiver<ConsoleEvent>,
    inputs: Sender<ConsoleInput>,
}

impl ConsoleBackend for ChannelBackend {
    fn poll_event(&mut self) -> Option<ConsoleEvent> {
        self.events.try_recv().ok()
    }

    fn send(&mut self, input: ConsoleInput) {
        let _ = self.inputs.send(input);
    }
}
