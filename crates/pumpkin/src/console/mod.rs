//! The full-screen terminal console, backed by the `pumpkin-tui` crate.
//!
//! Three pieces of wiring live here:
//!
//! * [`TuiLayer`] — a `tracing` layer that feeds the log pane instead of stdout.
//! * [`DispatcherCompleter`] — completion answered by the real command
//!   dispatcher, the same way the rustyline console does it.
//! * [`start`] — the console thread, the command loop, and the status feed.
//!
//! Everything is inert unless `commands.use_tui` is on and stdin is a TTY; the
//! readline console in `lib.rs` stays the default.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pumpkin_config::AdvancedConfiguration;
use pumpkin_data::packet::CURRENT_MC_VERSION;
use pumpkin_tui::backend::{ConsoleInput, Level as TuiLevel, LogRecord, PlayerInfo, ServerStatus};
use pumpkin_tui::channel::{ChannelBackend, ConsoleHandle};
use pumpkin_tui::completion::{
    Completer, Completion, CompletionKind, CompletionRequest, Completions,
};
use pumpkin_tui::{Console, ConsoleConfig};
use time::{OffsetDateTime, UtcOffset, format_description::FormatItem};
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::command::CommandSender;
use crate::command::string_reader::StringReader;
use crate::logging::StringVisitor;
use crate::plugin::server::server_command::ServerCommandEvent;
use crate::server::Server;
use crate::{SHOULD_STOP, STOP_INTERRUPT, stop_server};

/// Events the console can buffer before the server starts dropping them. The
/// queue only fills if the terminal cannot keep up; a dropped record is counted
/// and reported at shutdown rather than blocking a tick.
const EVENT_QUEUE: usize = 8_192;
/// How often the header and the player sidebar are refreshed.
const STATUS_INTERVAL: Duration = Duration::from_secs(1);

/// Set once the console owns the terminal, so the panic hook knows it has to
/// hand the terminal back before printing anything.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HANDLE: OnceLock<ConsoleHandle> = OnceLock::new();
/// Parked between `init_logger` (which needs the handle) and `start` (which
/// needs the console side of the same channel).
static BACKEND: Mutex<Option<ChannelBackend>> = Mutex::new(None);
/// How log records are stamped, shared by the logging layer and by command
/// replies so both columns line up.
static TIMESTAMP: OnceLock<Option<Stamp>> = OnceLock::new();

type Stamp = (UtcOffset, Vec<FormatItem<'static>>);

/// Whether the config asks for the TUI *and* the terminal can host it.
#[must_use]
pub fn wanted(advanced_config: &AdvancedConfiguration) -> bool {
    use std::io::{IsTerminal, stdin};

    advanced_config.commands.use_console
        && advanced_config.commands.use_tui
        && stdin().is_terminal()
}

/// Build the console channel and the logging layer that feeds it.
///
/// Called from `init_logger`, before a `Server` exists; the handle is stashed
/// globally so the ticker and the shutdown path can reach it later.
pub fn init(advanced_config: &AdvancedConfiguration) -> TuiLayer {
    let (handle, backend) = pumpkin_tui::console_channel(EVENT_QUEUE);
    if let Ok(mut slot) = BACKEND.lock() {
        *slot = Some(backend);
    }
    let _ = HANDLE.set(handle.clone());

    let timestamp = advanced_config.logging.timestamp.then(|| {
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let format: &'static str = Box::leak(
            advanced_config
                .logging
                .timestamp_format
                .clone()
                .into_boxed_str(),
        );
        let items = time::format_description::parse(format).unwrap_or_else(|_| {
            time::macros::format_description!("[hour]:[minute]:[second]").to_vec()
        });
        (offset, items)
    });
    let _ = TIMESTAMP.set(timestamp);

    TuiLayer {
        handle,
        show_target: advanced_config.logging.target,
    }
}

/// The current time in the configured log format, or an empty string when
/// timestamps are switched off.
fn now_stamp() -> String {
    let Some(Some((offset, format))) = TIMESTAMP.get() else {
        return String::new();
    };
    OffsetDateTime::now_utc()
        .to_offset(*offset)
        .format(format)
        .unwrap_or_default()
}

/// Show a console command's reply in the log pane.
///
/// Returns `false` when there is no console to show it in, so the caller can
/// fall back to printing on stdout. Without this, `println!` would draw
/// straight over the UI.
pub fn reply(message: &str) -> bool {
    let Some(handle) = HANDLE.get() else {
        return false;
    };

    let stamp = now_stamp();
    for line in strip_ansi(message).lines() {
        let _ =
            handle.log(LogRecord::new(TuiLevel::Info, line.to_owned()).with_time(stamp.clone()));
    }
    true
}

/// The console handle, once [`init`] has run.
#[must_use]
pub fn handle() -> Option<&'static ConsoleHandle> {
    HANDLE.get()
}

/// Whether the console currently owns the terminal.
#[must_use]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Hand the terminal back: leave the alternate screen and raw mode.
///
/// Safe to call from a panic hook, safe to call twice, and a no-op when the
/// console was never prepared.
pub fn restore_terminal() {
    if HANDLE.get().is_some() {
        ACTIVE.store(false, Ordering::Release);
        pumpkin_tui::restore_terminal();
    }
}

/// How long shutdown waits for the console thread to give the terminal back.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Close the console once the server has finished shutting down.
///
/// This blocks until the console thread has actually left the alternate screen:
/// the process usually exits within milliseconds of this call, and exiting
/// first would strand the terminal in raw mode.
pub fn shutdown() {
    let Some(handle) = HANDLE.get() else {
        return;
    };

    let dropped = handle.dropped_events();
    if dropped > 0 {
        tracing::debug!("Console dropped {dropped} event(s) while catching up");
    }
    let _ = handle.close();

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while is_active() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Belt and braces: if the console thread is wedged, take the terminal back
    // ourselves rather than leaving the operator with a dead shell.
    restore_terminal();
}

/// Start the console: the UI thread, the command loop, and the status feed.
///
/// Returns `false` when no console was prepared (the TUI is off), so the caller
/// can fall back to the readline console.
pub fn start(server: &Arc<Server>) -> bool {
    let Some(backend) = BACKEND.lock().ok().and_then(|mut slot| slot.take()) else {
        return false;
    };
    let Some(handle) = HANDLE.get().cloned() else {
        return false;
    };

    spawn_command_loop(server, handle.clone());
    spawn_status_feed(server, handle);
    spawn_ui_thread(server, backend);
    true
}

fn spawn_ui_thread(server: &Arc<Server>, backend: ChannelBackend) {
    let completer = Arc::new(DispatcherCompleter {
        server: server.clone(),
    });
    let config = ConsoleConfig {
        title: "Pumpkin".to_owned(),
        history_file: Some(std::path::PathBuf::from("logs/console_history")),
        min_level: TuiLevel::Info,
        // The command loop logs each submitted line through `tracing` instead,
        // so it is stamped and reaches the log file like everything else.
        echo_commands: false,
        ..ConsoleConfig::default()
    };

    let spawned = std::thread::Builder::new()
        .name("console".to_owned())
        .spawn(move || {
            ACTIVE.store(true, Ordering::Release);
            let result = Console::new(backend)
                .with_completer(completer)
                .with_config(config)
                .run();
            ACTIVE.store(false, Ordering::Release);

            if let Err(error) = result {
                tracing::error!("Console UI stopped: {error}");
            }
            // The operator closing the console must not leave the server
            // running headless.
            stop_server();
        });

    if let Err(error) = spawned {
        tracing::error!("Failed to start the console UI: {error}");
        stop_server();
    }
}

/// Feed submitted lines to the dispatcher, exactly as the stdin console does.
fn spawn_command_loop(server: &Arc<Server>, handle: ConsoleHandle) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

    let input_thread = std::thread::Builder::new()
        .name("console-input".to_owned())
        .spawn(move || {
            while !SHOULD_STOP.load(Ordering::Relaxed) {
                match handle.next_input_timeout(STATUS_INTERVAL) {
                    Some(ConsoleInput::Command(line)) => {
                        if tx.blocking_send(line).is_err() {
                            break;
                        }
                    }
                    Some(ConsoleInput::Quit) => {
                        stop_server();
                        break;
                    }
                    // Timed out: loop around and re-check the stop flag.
                    None => {}
                }
            }
        });
    if let Err(error) = input_thread {
        tracing::error!("Failed to start the console input thread: {error}");
    }

    let dispatch = server.clone();
    server.spawn_task(async move {
        while !SHOULD_STOP.load(Ordering::Relaxed) {
            let line = tokio::select! {
                line = rx.recv() => line,
                () = STOP_INTERRUPT.cancelled() => None,
            };
            let Some(line) = line else {
                break;
            };

            tracing::info!(target: "console", "> {line}");

            let mut event = ServerCommandEvent::new(line.clone());
            dispatch.plugin_manager.fire(&dispatch, &mut event).await;
            if !event.cancelled {
                dispatch
                    .command_dispatcher
                    .load()
                    .handle_command(&CommandSender::Console.into_source(&dispatch), &line);
            }
        }
        tracing::debug!("Stopped console command task");
    });
}

/// Push the header counters and the player sidebar once a second.
fn spawn_status_feed(server: &Arc<Server>, handle: ConsoleHandle) {
    let server = server.clone();
    server.clone().spawn_task(async move {
        let started = Instant::now();
        let mut system = sysinfo::System::new();
        let pid = sysinfo::get_current_pid().ok();

        while !SHOULD_STOP.load(Ordering::Relaxed) {
            tokio::select! {
                () = tokio::time::sleep(STATUS_INTERVAL) => {}
                () = STOP_INTERRUPT.cancelled() => break,
            }

            let players = server.get_all_players();
            let max_tps = f64::from(server.basic_config.tps);
            let status = ServerStatus {
                brand: "Pumpkin".to_owned(),
                version: CURRENT_MC_VERSION.to_string(),
                tps: server.get_tps().min(max_tps),
                mspt: server.get_mspt(),
                players_online: u32::try_from(players.len()).unwrap_or(u32::MAX),
                players_max: server.max_players(),
                memory_used_mb: memory_used_mb(&mut system, pid),
                memory_total_mb: system.total_memory() / BYTES_PER_MEBIBYTE,
                chunks_loaded: loaded_chunks(&server),
                uptime: started.elapsed(),
            };

            let sidebar = players
                .iter()
                .map(|player| PlayerInfo {
                    name: player.gameprofile.name.clone(),
                    world: player.world().dimension.minecraft_name.to_string(),
                    ping_ms: player.ping.load(Ordering::Relaxed),
                })
                .collect();

            if !handle.set_status(status) || !handle.set_players(sidebar) {
                break;
            }
        }
    });
}

const BYTES_PER_MEBIBYTE: u64 = 1024 * 1024;

/// Resident memory of this process, falling back to system-wide usage when the
/// platform will not tell us about ourselves.
fn memory_used_mb(system: &mut sysinfo::System, pid: Option<sysinfo::Pid>) -> u64 {
    system.refresh_memory();
    if let Some(pid) = pid {
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if let Some(process) = system.process(pid) {
            return process.memory() / BYTES_PER_MEBIBYTE;
        }
    }
    system.used_memory() / BYTES_PER_MEBIBYTE
}

fn loaded_chunks(server: &Server) -> u64 {
    server
        .worlds
        .load()
        .iter()
        .map(|world| u64::try_from(world.level.loaded_chunk_count()).unwrap_or(u64::MAX))
        .sum()
}

// ------------------------------------------------------------------ logging --

/// Feeds `tracing` events into the console's log pane.
pub struct TuiLayer {
    handle: ConsoleHandle,
    show_target: bool,
}

impl<S: Subscriber> Layer<S> for TuiLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = StringVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let mut record = LogRecord::new(level_of(*metadata.level()), strip_ansi(visitor.message()));
        if self.show_target {
            record = record.with_target(metadata.target());
        }
        let stamp = now_stamp();
        if !stamp.is_empty() {
            record = record.with_time(stamp);
        }

        let _ = self.handle.log(record);
    }
}

const fn level_of(level: tracing::Level) -> TuiLevel {
    match level {
        tracing::Level::TRACE => TuiLevel::Trace,
        tracing::Level::DEBUG => TuiLevel::Debug,
        tracing::Level::INFO => TuiLevel::Info,
        tracing::Level::WARN => TuiLevel::Warn,
        tracing::Level::ERROR => TuiLevel::Error,
    }
}

/// Drop ANSI escape sequences.
///
/// Chat components reach the log already coloured by
/// `TextComponent::to_pretty_console`; ratatui draws text verbatim, so those
/// bytes would otherwise show up as literal `\u{1b}[38;5;…` noise. The log pane
/// colours by level instead.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI (`ESC [`) runs until a byte in 0x40..=0x7E; anything else is a
        // two-character escape.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

// --------------------------------------------------------------- completion --

/// Answers the console's completion queries from the live command dispatcher.
struct DispatcherCompleter {
    server: Arc<Server>,
}

impl Completer for DispatcherCompleter {
    fn complete(&self, request: CompletionRequest<'_>) -> Completions {
        let prefix = request.prefix();
        let has_slash = usize::from(prefix.starts_with('/'));
        let typed = &prefix[has_slash..];

        let dispatcher = self.server.command_dispatcher.load();
        let source = CommandSender::Console.into_source(&self.server);

        if typed.trim().is_empty() {
            // `get_all_commands` is a `BTreeMap`, so this comes out sorted with
            // each command's description attached.
            let items = dispatcher
                .get_all_commands()
                .into_iter()
                .map(|(name, description)| {
                    Completion::new(name, CompletionKind::Command).with_detail(description)
                })
                .collect();
            return Completions::new(prefix.len(), items);
        }

        let Some(cursor) = request.cursor.checked_sub(has_slash) else {
            return Completions::default();
        };

        let mut reader = StringReader::new(typed);
        let parsed = dispatcher.parse(&mut reader, &source);
        let suggestions = dispatcher.get_completion_suggestions(parsed, cursor);
        if suggestions.is_empty() {
            return Completions::default();
        }

        let online: Vec<String> = self
            .server
            .get_all_players()
            .iter()
            .map(|player| player.gameprofile.name.clone())
            .collect();

        let items = suggestions
            .suggestions
            .into_iter()
            .map(|suggestion| {
                let text = suggestion.text.cached_text().clone();
                let kind = if online.contains(&text) {
                    CompletionKind::Player
                } else {
                    CompletionKind::Argument
                };
                Completion::new(text, kind)
            })
            .collect();

        Completions::new(suggestions.range.start + has_slash, items)
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_colour_escapes_from_chat_components() {
        assert_eq!(
            strip_ansi("\u{1b}[38;2;255;170;0mNotch\u{1b}[0m joined"),
            "Notch joined"
        );
        assert_eq!(strip_ansi("plain text"), "plain text");
        // A truncated escape must not swallow the rest of the line silently.
        assert_eq!(strip_ansi("a\u{1b}"), "a");
    }
}
