//! The log pane.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;
use tracing::Level;

use super::Theme;
use crate::tui::app::App;

/// The `L` pane: the newest log lines that fit.
pub(super) fn log_pane(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" Log (L to hide) ", theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let rows = usize::from(block.inner(area).height);
    let lines: Vec<Line> = app
        .log
        .tail(rows)
        .into_iter()
        .map(|line| {
            let level = match line.level {
                Level::ERROR => Span::styled("ERROR", theme.danger()),
                Level::WARN => Span::styled("WARN ", theme.warn()),
                Level::INFO => Span::raw("INFO "),
                Level::DEBUG => Span::styled("DEBUG", theme.dim()),
                _ => Span::styled("TRACE", theme.dim()),
            };
            Line::from(vec![
                Span::styled(format!("{} ", line.time), theme.dim()),
                level,
                Span::raw(format!(" {}", line.text)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
