//! The `?` overlay (#1895): the sections of [`crate::tui::help`] for the
//! screen shown, scrolled with the movement keys when they don't fit.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use super::{Theme, centered, text_width};
use crate::tui::app::App;
use crate::tui::help::sections;

/// The widest the box gets.
const MAX_WIDTH: u16 = 84;
/// Columns before a row's description: the indent and the keys.
const KEY_COLUMN: usize = 20;

/// Draw the help overlay over `area`.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // A column clear each side, so the border never doubles a pane's.
    let width = area.width.saturating_sub(4).min(MAX_WIDTH);
    let inner = usize::from(width.saturating_sub(2));
    let lines = lines(app, inner, theme);
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rows = usize::from(height.saturating_sub(2)).max(1);
    // Clamp the scroll to the last page, and keep the clamped value so Up
    // moves from where the view is.
    let last = lines.len().saturating_sub(rows);
    let top = app.help_scroll.get().min(last);
    app.help_scroll.set(top);
    let box_area = centered(area, width, height);
    frame.render_widget(Clear, box_area);
    let mut block = Block::bordered()
        .title(Line::styled(" Help (? or Esc to close) ", theme.accent()))
        .border_style(theme.accent());
    if last > 0 {
        let shown = (top + rows).min(lines.len());
        block = block.title_bottom(
            Line::styled(
                format!(
                    " {}-{shown} of {}  Up Down PgUp PgDn scroll ",
                    top + 1,
                    lines.len()
                ),
                theme.dim(),
            )
            .right_aligned(),
        );
    }
    let shown: Vec<Line> = lines.into_iter().skip(top).take(rows).collect();
    frame.render_widget(Paragraph::new(shown).block(block), box_area);
}

/// Every line of the help, wrapped to `width` columns.
fn lines(app: &App, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for section in sections(app) {
        out.push(Line::styled(format!(" {}", section.title), theme.accent()));
        for (keys, what) in section.rows {
            let lead = format!("   {keys}");
            // A space before the right border.
            let room = width.saturating_sub(KEY_COLUMN + 1).max(10);
            let mut wrapped = wrap(&what, room).into_iter();
            // Keys too wide for their column get a line of their own.
            if text_width(&lead) < KEY_COLUMN {
                let first = wrapped.next().unwrap_or_default();
                out.push(Line::from(vec![
                    Span::styled(format!("{lead:<KEY_COLUMN$}"), theme.warn()),
                    Span::raw(first),
                ]));
            } else {
                out.push(Line::styled(lead, theme.warn()));
            }
            for rest in wrapped {
                out.push(Line::raw(format!("{:KEY_COLUMN$}{rest}", "")));
            }
        }
    }
    out
}

/// `text` in lines of at most `width` columns, broken between words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && text_width(&line) + 1 + text_width(word) > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    lines
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn descriptions_break_between_words() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("unbreakable", 4), ["unbreakable"]);
        assert_eq!(wrap("", 4), [""]);
    }
}
