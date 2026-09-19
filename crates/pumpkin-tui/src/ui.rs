//! Rendering. Pure functions of [`App`] state — nothing here mutates the
//! console except to record pane geometry and clamp the scroll offset.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Sparkline};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};
use crate::backend::{Level, LogRecord, PlayerInfo};
use crate::theme::Theme;
use crate::wrap::{find_matches, wrap};

const SIDEBAR_WIDTH: u16 = 30;
const POPUP_MAX_ROWS: usize = 8;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(3),    // log + sidebar
        Constraint::Length(3), // prompt
        Constraint::Length(1), // footer
    ])
    .split(area);

    draw_header(frame, app, rows[0]);
    draw_body(frame, app, rows[1]);
    draw_prompt(frame, app, rows[2]);
    draw_footer(frame, app, rows[3]);

    if app.completion.open && app.mode == Mode::Normal {
        draw_completions(frame, app, rows[2]);
    }
    match app.mode {
        Mode::Help => draw_help(frame, app, area),
        Mode::ConfirmQuit => draw_confirm(frame, app, area),
        Mode::Normal | Mode::Search => {}
    }
}

fn block(theme: &Theme) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
}

// ---------------------------------------------------------------- header ---

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let status = &app.status;

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(app.config.title.clone(), theme.accent_style()),
        Span::raw(" "),
    ]);
    let outer = block(theme).title_top(title);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Counters on the left, an MSPT sparkline on the right with a gap between
    // them so a long memory readout never runs into the bars.
    let columns = Layout::horizontal([
        Constraint::Min(20),
        Constraint::Length(2),
        Constraint::Length(if inner.width > 70 { 14 } else { 0 }),
    ])
    .split(inner);

    let mut spans = vec![Span::raw(" ")];
    if !status.version.is_empty() {
        spans.push(Span::styled(
            status.version.clone(),
            Style::default().fg(theme.accent_dim),
        ));
        spans.push(separator(theme));
    }
    spans.push(Span::styled("up ", theme.muted_style()));
    spans.push(Span::raw(format_uptime(status.uptime)));
    spans.push(separator(theme));

    spans.push(Span::styled("players ", theme.muted_style()));
    spans.push(Span::styled(
        format!("{}/{}", status.players_online, status.players_max),
        Style::default().fg(theme.good),
    ));
    spans.push(separator(theme));

    spans.push(Span::styled("tps ", theme.muted_style()));
    spans.push(Span::styled(
        format!("{:.2}", status.tps),
        Style::default().fg(theme.health_color(20.0 - status.tps, 0.5, 2.0)),
    ));
    spans.push(separator(theme));

    spans.push(Span::styled("mspt ", theme.muted_style()));
    spans.push(Span::styled(
        format!("{:.1}", status.mspt),
        Style::default().fg(theme.health_color(status.mspt, 35.0, 50.0)),
    ));
    spans.push(separator(theme));

    spans.push(Span::styled("mem ", theme.muted_style()));
    spans.push(Span::raw(format!(
        "{}/{} MB",
        status.memory_used_mb, status.memory_total_mb
    )));

    if status.chunks_loaded > 0 {
        spans.push(separator(theme));
        spans.push(Span::styled("chunks ", theme.muted_style()));
        spans.push(Span::raw(status.chunks_loaded.to_string()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), columns[0]);

    let chart = columns[2];
    if !app.mspt_samples.is_empty() && chart.width > 0 {
        let width = usize::from(chart.width);
        let samples: Vec<u64> = app
            .mspt_samples
            .iter()
            .rev()
            .take(width)
            .rev()
            .copied()
            .collect();
        frame.render_widget(
            Sparkline::default()
                .data(&samples)
                .max(50)
                .style(Style::default().fg(theme.accent_dim)),
            chart,
        );
    }
}

fn separator(theme: &Theme) -> Span<'static> {
    Span::styled("  │  ", Style::default().fg(theme.border))
}

fn format_uptime(uptime: std::time::Duration) -> String {
    let total = uptime.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

// ------------------------------------------------------------------ body ---

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let show_sidebar = app.show_players && area.width > SIDEBAR_WIDTH + 40;
    let panes = if show_sidebar {
        Layout::horizontal([Constraint::Min(40), Constraint::Length(SIDEBAR_WIDTH)]).split(area)
    } else {
        Layout::horizontal([Constraint::Min(0)]).split(area)
    };

    draw_log(frame, app, panes[0]);
    if show_sidebar {
        draw_players(frame, app, panes[1]);
    }
}

fn draw_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let mut title_right = Vec::new();
    let level = app.min_level.label().trim().to_owned();
    title_right.push(Span::styled(
        format!(" ≥{level} "),
        Style::default().fg(theme.level_color(app.min_level)),
    ));
    if !app.search.text().is_empty() {
        title_right.push(Span::styled(
            format!("/{} ", app.search.text()),
            Style::default().fg(theme.match_fg),
        ));
    }
    if !app.is_following() {
        title_right.push(Span::styled(
            "PAUSED ",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ));
    }

    let outer = block(&theme)
        .title_top(Line::from(vec![
            Span::raw(" "),
            Span::styled("log", theme.accent_style()),
            Span::raw(" "),
        ]))
        .title_top(Line::from(title_right).right_aligned());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    app.log_area = inner;

    let height = usize::from(inner.height);
    let width = usize::from(inner.width);
    if height == 0 || width == 0 {
        return;
    }

    let (lines, more_above) = collect_lines(app, width, height);
    frame.render_widget(Paragraph::new(lines), inner);

    if more_above > 0 {
        let hint = format!(" ↑ {more_above} more ");
        let hint_width = u16::try_from(hint.width()).unwrap_or(0).min(inner.width);
        let rect = Rect {
            x: inner.x + inner.width.saturating_sub(hint_width),
            y: inner.y,
            width: hint_width,
            height: 1,
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(theme.muted))),
            rect,
        );
    }
}

/// Render the visible slice of the log, newest-last, clamping `app.scroll` to
/// however many wrapped lines actually exist above the viewport.
fn collect_lines(app: &mut App, width: usize, height: usize) -> (Vec<Line<'static>>, usize) {
    let needed = app.scroll.saturating_add(height);
    let query = app.search.text().to_owned();
    let theme = app.theme;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut exhausted = true;
    for record in app.visible_records().rev() {
        let mut rendered = render_record(record, width, &query, &theme);
        rendered.append(&mut lines);
        lines = rendered;
        if lines.len() >= needed {
            exhausted = false;
            break;
        }
    }

    // Fewer lines than the operator scrolled for: pin to the oldest record.
    if exhausted && lines.len() < needed {
        app.scroll = lines.len().saturating_sub(height);
    }

    let end = lines.len().saturating_sub(app.scroll);
    let start = end.saturating_sub(height);
    let more_above = start;

    // Anchor the tail to the bottom of the pane, the way a terminal does, so
    // fresh output always appears just above the prompt.
    let mut visible = vec![Line::default(); height.saturating_sub(end - start)];
    visible.extend_from_slice(&lines[start..end]);
    (visible, more_above)
}

fn render_record(
    record: &LogRecord,
    width: usize,
    query: &str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let time_width = record.time.width();
    let gutter = if record.time.is_empty() {
        6
    } else {
        time_width + 7
    };
    let body_width = width.saturating_sub(gutter).max(8);

    let target_prefix = record
        .target
        .as_ref()
        .map(|target| format!("{target}: "))
        .unwrap_or_default();
    let body = format!("{target_prefix}{}", record.message);
    let wrapped = wrap(&body, body_width);

    let message_style = match record.level {
        Level::Error | Level::Warn => Style::default().fg(theme.level_color(record.level)),
        Level::Trace | Level::Debug => Style::default().fg(theme.muted),
        Level::Info => Style::default().fg(theme.text),
    };

    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let mut spans = Vec::new();
            if index == 0 {
                if !record.time.is_empty() {
                    spans.push(Span::styled(record.time.clone(), theme.muted_style()));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    record.level.label().to_owned(),
                    theme.level_style(record.level),
                ));
                spans.push(Span::raw(" "));

                // Dim the `target:` prefix, but only while it survived wrapping
                // intact on this line.
                if !target_prefix.is_empty() && text.starts_with(&target_prefix) {
                    spans.push(Span::styled(
                        target_prefix.clone(),
                        Style::default().fg(theme.accent_dim),
                    ));
                    spans.extend(highlight(
                        &text[target_prefix.len()..],
                        query,
                        message_style,
                        theme,
                    ));
                    return Line::from(spans);
                }
            } else {
                spans.push(Span::raw(" ".repeat(gutter)));
            }
            spans.extend(highlight(&text, query, message_style, theme));
            Line::from(spans)
        })
        .collect()
}

/// Split `text` into spans so search matches stand out.
fn highlight(text: &str, query: &str, base: Style, theme: &Theme) -> Vec<Span<'static>> {
    let matches = find_matches(text, query);
    if matches.is_empty() {
        return vec![Span::styled(text.to_owned(), base)];
    }

    let highlighted = Style::default().fg(theme.match_fg).bg(theme.match_bg);
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (start, end) in matches {
        if start > cursor {
            spans.push(Span::styled(text[cursor..start].to_owned(), base));
        }
        spans.push(Span::styled(text[start..end].to_owned(), highlighted));
        cursor = end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), base));
    }
    spans
}

fn draw_players(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let outer = block(theme).title_top(Line::from(vec![
        Span::raw(" "),
        Span::styled("players", theme.accent_style()),
        Span::styled(format!(" {} ", app.players.len()), theme.muted_style()),
    ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if app.players.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("nobody online", theme.muted_style())),
            inner,
        );
        return;
    }

    let lines: Vec<Line<'static>> = app
        .players
        .iter()
        .map(|player| player_line(player, theme))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn player_line(player: &PlayerInfo, theme: &Theme) -> Line<'static> {
    let ping_color = theme.health_color(f64::from(player.ping_ms), 80.0, 200.0);
    Line::from(vec![
        Span::styled("● ", Style::default().fg(ping_color)),
        Span::raw(player.name.clone()),
        Span::styled(format!(" {}", player.world), theme.muted_style()),
        Span::styled(
            format!(" {}ms", player.ping_ms),
            Style::default().fg(ping_color),
        ),
    ])
}

// ---------------------------------------------------------------- prompt ---

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let searching = app.mode == Mode::Search;
    let editor = if searching { &app.search } else { &app.editor };

    let label = if searching { "search" } else { "command" };
    let outer = block(theme)
        .border_style(Style::default().fg(if searching {
            theme.match_fg
        } else {
            theme.accent_dim
        }))
        .title_top(Line::from(vec![
            Span::raw(" "),
            Span::styled(label, theme.accent_style()),
            Span::raw(" "),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let marker = if searching {
        "/".to_owned()
    } else {
        format!("{} ", app.config.prompt)
    };
    let marker_width = marker.width();

    let mut spans = vec![
        Span::styled(marker, Style::default().fg(theme.accent)),
        Span::raw(editor.text().to_owned()),
    ];
    if !searching && let Some(hint) = &app.completion.hint {
        spans.push(Span::styled(
            hint.clone(),
            Style::default().fg(theme.muted).add_modifier(Modifier::DIM),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);

    let cursor_x =
        inner.x + u16::try_from(marker_width + editor.cursor_column()).unwrap_or(u16::MAX);
    frame.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), inner.y));
}

fn draw_completions(frame: &mut Frame, app: &App, prompt_area: Rect) {
    let theme = &app.theme;
    let items = &app.completion.items.items;
    if items.is_empty() {
        return;
    }

    // Scroll the popup so the selection stays visible.
    let rows = items.len().min(POPUP_MAX_ROWS);
    let first = app
        .completion
        .selected
        .saturating_sub(rows.saturating_sub(1))
        .min(items.len().saturating_sub(rows));

    let value_width = items
        .iter()
        .map(|item| item.value.width())
        .max()
        .unwrap_or(0);
    let detail_width = items
        .iter()
        .filter_map(|item| item.detail.as_ref().map(|detail| detail.width() + 2))
        .max()
        .unwrap_or(0);
    let content_width = value_width + detail_width + 2;

    let anchor = usize::from(prompt_area.x)
        + 1
        + app.config.prompt.width()
        + 1
        + app.editor.text()[..app.completion.items.start.min(app.editor.text().len())].width();

    let width = u16::try_from(content_width + 2)
        .unwrap_or(u16::MAX)
        .min(prompt_area.width);
    let height = u16::try_from(rows + 2).unwrap_or(u16::MAX);
    let x = u16::try_from(anchor)
        .unwrap_or(prompt_area.x)
        .min(prompt_area.right().saturating_sub(width));
    let y = prompt_area.y.saturating_sub(height);

    let area = Rect {
        x,
        y,
        width,
        height,
    };

    let hidden = items.len().saturating_sub(rows);
    let mut popup = block(theme).border_style(Style::default().fg(theme.accent_dim));
    if hidden > 0 {
        popup = popup.title_bottom(
            Line::from(Span::styled(
                format!(" +{hidden} more "),
                theme.muted_style(),
            ))
            .right_aligned(),
        );
    }

    let lines: Vec<Line<'static>> = items
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(index, item)| {
            let selected = index == app.completion.selected;
            let style = if selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.completion_color(item.kind))
            };

            let mut spans = vec![
                Span::styled(if selected { "▌" } else { " " }.to_owned(), style),
                Span::styled(item.value.clone(), style),
            ];
            if let Some(detail) = &item.detail {
                spans.push(Span::styled(
                    format!("  {detail}"),
                    if selected { style } else { theme.muted_style() },
                ));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(popup), area);
}

// ---------------------------------------------------------------- footer ---

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    if let Some(notice) = app.notice_text() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ● ", Style::default().fg(theme.accent)),
                Span::styled(notice.to_owned(), Style::default().fg(theme.text)),
            ])),
            area,
        );
        return;
    }

    let hints: [(&str, &str); 6] = [
        ("Tab", "complete"),
        ("↑↓", "history"),
        ("F1", "help"),
        ("F2", "players"),
        ("F3", "search"),
        ("F4", "level"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
        spans.push(Span::styled(
            key.to_owned(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}   "), theme.muted_style()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// -------------------------------------------------------------- overlays ---

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let bindings: [(&str, &str); 15] = [
        (
            "Enter",
            "run the command (or accept the highlighted candidate)",
        ),
        ("Tab / Shift+Tab", "complete, then cycle through candidates"),
        ("→ / End", "accept the dimmed inline suggestion"),
        ("↑ / ↓", "command history"),
        ("Ctrl+A / Ctrl+E", "start / end of line"),
        (
            "Ctrl+W / Ctrl+U / Ctrl+K",
            "delete word / to start / to end",
        ),
        ("PgUp / PgDn", "scroll the log a page"),
        ("Shift+↑ / Shift+↓", "scroll the log a line"),
        ("Ctrl+End", "jump back to the newest line"),
        ("Ctrl+L", "clear the log pane"),
        ("F2", "toggle the player sidebar"),
        ("F3", "filter the log (Esc clears)"),
        ("F4", "cycle the minimum log level"),
        ("F5", "follow the tail again"),
        ("Ctrl+C / Ctrl+D", "quit the console"),
    ];

    let key_width = bindings
        .iter()
        .map(|(key, _)| key.width())
        .max()
        .unwrap_or(0);
    let lines: Vec<Line<'static>> = bindings
        .iter()
        .map(|(key, description)| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{key:<key_width$}"),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled((*description).to_owned(), Style::default().fg(theme.text)),
            ])
        })
        .collect();

    let width = u16::try_from(key_width + 60).unwrap_or(80);
    let height = u16::try_from(lines.len() + 2).unwrap_or(20);
    let area = centered(area, width, height);
    let popup = block(theme)
        .border_style(Style::default().fg(theme.accent))
        .title_top(Line::from(vec![
            Span::raw(" "),
            Span::styled("keys", theme.accent_style()),
            Span::raw(" "),
        ]))
        .title_bottom(
            Line::from(Span::styled(" any key closes ", theme.muted_style())).right_aligned(),
        );

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(popup), area);
}

fn draw_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let area = centered(area, 52, 5);
    let popup = block(theme).border_style(Style::default().fg(theme.warn));

    let lines = vec![
        Line::from(Span::styled(
            "Leave the console?",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "The server is told to shut down cleanly.",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("y", theme.accent_style()),
            Span::styled(" quit    ", theme.muted_style()),
            Span::styled("n", theme.accent_style()),
            Span::styled(" stay", theme.muted_style()),
        ]),
    ];

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(popup), area);
}

#[cfg(test)]
mod tests {
    use super::draw;
    use crate::app::{App, ConsoleConfig};
    use crate::backend::{Level, LogRecord, PlayerInfo, ServerStatus};
    use crate::completion::{CommandTree, Node};
    use crate::theme::Theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;
    use std::time::Duration;

    fn app() -> App {
        let tree = CommandTree::new().with(
            Node::literal("gamemode")
                .detail("Change a player's game mode")
                .then(Node::argument("<mode>").suggest(["survival", "creative"])),
        );
        let mut app = App::new(ConsoleConfig::default(), Theme::default(), Arc::new(tree));
        app.status = ServerStatus {
            version: "1.21.9".to_owned(),
            players_online: 1,
            players_max: 20,
            uptime: Duration::from_secs(3_725),
            ..ServerStatus::default()
        };
        app.players.push(PlayerInfo {
            name: "Notch".to_owned(),
            world: "overworld".to_owned(),
            ping_ms: 42,
        });
        app.push_record(
            LogRecord::new(Level::Info, "Notch joined the game")
                .with_time("12:03:41")
                .with_target("net"),
        );
        app
    }

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn renders_the_full_layout() {
        let mut app = app();
        let screen = render(&mut app, 120, 30);
        assert!(screen.contains("Pumpkin"));
        assert!(screen.contains("Notch joined the game"));
        assert!(screen.contains("players"));
        assert!(screen.contains("01:02:05")); // uptime
    }

    #[test]
    fn renders_the_completion_popup() {
        let mut app = app();
        app.editor.set("gamemode ");
        app.completion.items = crate::completion::Completer::complete(
            app.completer.as_ref(),
            crate::completion::CompletionRequest {
                line: "gamemode ",
                cursor: 9,
            },
        );
        app.completion.open = true;
        let screen = render(&mut app, 120, 30);
        assert!(screen.contains("survival"));
        assert!(screen.contains("creative"));
    }

    #[test]
    fn survives_tiny_terminals() {
        let mut app = app();
        for (width, height) in [(20, 8), (10, 4), (1, 1), (200, 60)] {
            let _ = render(&mut app, width, height);
        }
    }

    #[test]
    fn clamps_scroll_past_the_oldest_record() {
        let mut app = app();
        app.scroll = 10_000;
        let _ = render(&mut app, 80, 20);
        assert_eq!(
            app.scroll, 0,
            "scrolling past the top pins to the oldest line"
        );
    }

    #[test]
    fn the_newest_line_sits_at_the_bottom_of_the_pane() {
        let mut app = app();
        let (lines, above) = super::collect_lines(&mut app, 60, 5);
        assert_eq!(lines.len(), 5);
        assert_eq!(above, 0);
        assert!(lines[4].to_string().contains("Notch joined the game"));
        assert!(lines[0].to_string().trim().is_empty());
    }
}
