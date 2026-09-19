//! Colours. Everything the UI draws pulls from here, so a host can restyle the
//! console without touching layout code.

use ratatui::style::{Color, Modifier, Style};

use crate::backend::Level;
use crate::completion::CompletionKind;

/// The console's palette.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub accent: Color,
    pub accent_dim: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub trace: Color,
    pub debug: Color,
    pub info: Color,
    pub warn: Color,
    pub error: Color,
    pub good: Color,
    pub bad: Color,
    pub popup_bg: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub match_bg: Color,
    pub match_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::pumpkin()
    }
}

impl Theme {
    /// Orange on dark — the default.
    #[must_use]
    pub const fn pumpkin() -> Self {
        Self {
            accent: Color::Rgb(255, 150, 45),
            accent_dim: Color::Rgb(150, 92, 32),
            border: Color::Rgb(78, 70, 64),
            text: Color::Reset,
            muted: Color::Rgb(128, 122, 116),
            trace: Color::Rgb(120, 120, 130),
            debug: Color::Rgb(120, 170, 200),
            info: Color::Rgb(150, 200, 150),
            warn: Color::Rgb(230, 190, 80),
            error: Color::Rgb(235, 100, 95),
            good: Color::Rgb(130, 200, 130),
            bad: Color::Rgb(235, 100, 95),
            popup_bg: Color::Rgb(30, 28, 26),
            selection_bg: Color::Rgb(255, 150, 45),
            selection_fg: Color::Rgb(20, 18, 16),
            match_bg: Color::Rgb(90, 80, 40),
            match_fg: Color::Rgb(255, 230, 160),
        }
    }

    /// A palette that only uses the terminal's own 16 colours, for terminals
    /// (or ssh sessions) where truecolor is not available.
    #[must_use]
    pub const fn ansi() -> Self {
        Self {
            accent: Color::Yellow,
            accent_dim: Color::LightYellow,
            border: Color::DarkGray,
            text: Color::Reset,
            muted: Color::DarkGray,
            trace: Color::DarkGray,
            debug: Color::Cyan,
            info: Color::Green,
            warn: Color::Yellow,
            error: Color::Red,
            good: Color::Green,
            bad: Color::Red,
            popup_bg: Color::Black,
            selection_bg: Color::Yellow,
            selection_fg: Color::Black,
            match_bg: Color::Blue,
            match_fg: Color::White,
        }
    }

    #[must_use]
    pub const fn level_color(&self, level: Level) -> Color {
        match level {
            Level::Trace => self.trace,
            Level::Debug => self.debug,
            Level::Info => self.info,
            Level::Warn => self.warn,
            Level::Error => self.error,
        }
    }

    #[must_use]
    pub fn level_style(&self, level: Level) -> Style {
        let style = Style::default().fg(self.level_color(level));
        if matches!(level, Level::Error | Level::Warn) {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    #[must_use]
    pub const fn completion_color(&self, kind: CompletionKind) -> Color {
        match kind {
            CompletionKind::Command => self.accent,
            CompletionKind::Literal => self.debug,
            CompletionKind::Argument => self.info,
            CompletionKind::Player => self.good,
            CompletionKind::Placeholder => self.muted,
        }
    }

    #[must_use]
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    #[must_use]
    pub fn accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Colour for a "higher is worse" reading such as MSPT.
    #[must_use]
    pub const fn health_color(&self, value: f64, warn_above: f64, bad_above: f64) -> Color {
        if value > bad_above {
            self.bad
        } else if value > warn_above {
            self.warn
        } else {
            self.good
        }
    }
}
