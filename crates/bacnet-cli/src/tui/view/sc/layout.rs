//! Fitting the SC screen to its content (#1890): word wrap with hanging
//! indents, so a pane's height is known before it is drawn, and the split of
//! the top row between the hub connector and the identity panel.

use ratatui::text::{Line, Span};

/// A line and the column its wrapped continuation lines start at.
#[derive(Debug, Clone)]
pub(in crate::tui::view) struct Row<'a> {
    /// The line as written.
    pub(in crate::tui::view) line: Line<'a>,
    /// Where continuation lines start.
    pub(in crate::tui::view) indent: usize,
}

impl<'a> Row<'a> {
    /// A row whose continuation lines start at column `indent`.
    pub(in crate::tui::view) fn hang(line: impl Into<Line<'a>>, indent: usize) -> Self {
        Self {
            line: line.into(),
            indent,
        }
    }

    /// A row that wraps to the left edge.
    pub(in crate::tui::view) fn flow(line: impl Into<Line<'a>>) -> Self {
        Self::hang(line, 0)
    }
}

/// Columns `text` takes on screen.
pub(super) fn cols(text: &str) -> usize {
    Span::raw(text).width()
}

/// The widest row, unwrapped.
pub(super) fn natural(rows: &[Row]) -> usize {
    rows.iter().map(|row| row.line.width()).max().unwrap_or(0)
}

/// Words with the spaces after them.
fn pieces(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (at, ch) in text.char_indices() {
        if ch == ' ' {
            in_space = true;
        } else if in_space {
            out.push(&text[start..at]);
            start = at;
            in_space = false;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Where to split a word too wide for any line, with `room` columns left:
/// after the last hyphen that fits (a UUID breaks between its groups), else
/// at the edge; at least one character.
fn split_at(word: &str, room: usize) -> usize {
    let fits = word
        .char_indices()
        .scan(0, |taken, (at, ch)| {
            *taken += cols(&ch.to_string());
            Some((at + ch.len_utf8(), *taken))
        })
        .take_while(|(_, taken)| *taken <= room)
        .last()
        .map_or(0, |(end, _)| end);
    let hyphen = word[..fits].rfind('-').map(|at| at + 1);
    match hyphen.unwrap_or(fits) {
        0 => word.chars().next().map_or(0, char::len_utf8),
        cut => cut,
    }
}

/// One output line being filled.
struct Filling<'a> {
    lines: Vec<Line<'a>>,
    spans: Vec<Span<'a>>,
    used: usize,
}

impl<'a> Filling<'a> {
    fn push(&mut self, text: String, style: ratatui::style::Style) {
        self.used += cols(&text);
        self.spans.push(Span::styled(text, style));
    }

    /// End the line, dropping its trailing spaces, and start the next one
    /// at `indent`.
    fn next(&mut self, indent: usize) {
        if let Some(last) = self.spans.last_mut() {
            let trimmed = last.content.trim_end().to_string();
            last.content = trimmed.into();
        }
        self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        self.spans.push(Span::raw(" ".repeat(indent)));
        self.used = indent;
    }
}

/// `row` wrapped to `width` columns: words move whole to the next line,
/// which starts at the row's indent; a word wider than any line is split.
pub(super) fn wrap<'a>(row: &Row<'a>, width: usize) -> Vec<Line<'a>> {
    let width = width.max(1);
    // An indent that leaves no room is dropped.
    let indent = if row.indent + 8 <= width {
        row.indent
    } else {
        0
    };
    if row.line.width() <= width {
        return vec![row.line.clone()];
    }
    let mut fill = Filling {
        lines: Vec::new(),
        spans: Vec::new(),
        used: 0,
    };
    // Nothing on the current line yet but its indent.
    let mut fresh = true;
    for span in &row.line.spans {
        for piece in pieces(&span.content) {
            let word = cols(piece.trim_end());
            // A word no line holds is split where it stands, unless that
            // leaves it only a sliver of this line.
            let long = indent + word > width;
            let sliver = width.saturating_sub(fill.used) < 8;
            if !fresh && fill.used + word > width && (!long || sliver) {
                fill.next(indent);
                fresh = true;
            }
            if fresh && word == 0 && !fill.lines.is_empty() {
                // No leading spaces on a continuation line.
                continue;
            }
            let mut rest = piece;
            while fill.used + cols(rest.trim_end()) > width {
                let cut = split_at(rest, width.saturating_sub(fill.used));
                fill.push(rest[..cut].to_string(), span.style);
                rest = &rest[cut..];
                fill.next(indent);
            }
            if !rest.is_empty() {
                fill.push(rest.to_string(), span.style);
                fresh = false;
            }
        }
    }
    if fill
        .spans
        .iter()
        .any(|span| !span.content.trim().is_empty())
    {
        fill.next(0);
    }
    fill.lines
}

/// Every row wrapped to `width`.
pub(super) fn wrap_all<'a>(rows: &[Row<'a>], width: usize) -> Vec<Line<'a>> {
    rows.iter().flat_map(|row| wrap(row, width)).collect()
}

/// Lines `rows` take at `width`.
pub(super) fn height(rows: &[Row], width: usize) -> usize {
    wrap_all(rows, width).len()
}

/// The narrowest a top panel gets, borders included.
const PANEL_MIN: u16 = 24;

/// Split `width` between two bordered panels side by side: the right one's
/// width and the taller one's height, borders included. Both get their
/// natural width when that fits, the left one any spare; otherwise the
/// split with the fewest lines, the widest right panel among equals.
pub(super) fn split(width: u16, left: &[Row], right: &[Row]) -> (u16, u16) {
    let fit = |rows: &[Row], outer: u16| height(rows, usize::from(outer.saturating_sub(2)));
    let natural_right = u16::try_from(natural(right) + 2).unwrap_or(u16::MAX);
    let natural_left = u16::try_from(natural(left) + 2).unwrap_or(u16::MAX);
    let tall = |right_width: u16| {
        let lines = fit(left, width - right_width).max(fit(right, right_width));
        u16::try_from(lines + 2).unwrap_or(u16::MAX)
    };
    if natural_left.saturating_add(natural_right) <= width {
        return (natural_right, tall(natural_right));
    }
    let widest = natural_right.min(width.saturating_sub(PANEL_MIN));
    let mut best = (widest, tall(widest));
    for right_width in (PANEL_MIN..widest).rev() {
        let height = tall(right_width);
        if height < best.1 {
            best = (right_width, height);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn words_move_whole_and_continuations_hang() {
        let row = Row::hang(
            "Heartbeat  last heard 0 s ago, no heartbeat answered yet",
            11,
        );
        assert_eq!(
            text(&wrap(&row, 40)),
            [
                "Heartbeat  last heard 0 s ago, no",
                "           heartbeat answered yet",
            ]
        );
    }

    #[test]
    fn a_uuid_breaks_between_its_groups() {
        let row = Row::hang("Device UUID 784f3147-2ed1-476d-8a0c-0123456789ab", 12);
        assert_eq!(
            text(&wrap(&row, 35)),
            [
                "Device UUID 784f3147-2ed1-476d-",
                "            8a0c-0123456789ab",
            ]
        );
    }

    #[test]
    fn a_word_wider_than_the_line_is_split() {
        let row = Row::flow("abcdefghij klm");
        assert_eq!(text(&wrap(&row, 4)), ["abcd", "efgh", "ij", "klm"]);
    }

    #[test]
    fn a_short_row_is_left_alone() {
        let row = Row::flow("State connected");
        assert_eq!(text(&wrap(&row, 80)), ["State connected"]);
    }

    #[test]
    fn panels_take_their_natural_widths_when_they_fit() {
        let left = [Row::flow("x".repeat(40))];
        let right = [Row::flow("y".repeat(30))];
        assert_eq!(split(120, &left, &right), (32, 3));
    }

    #[test]
    fn a_narrow_split_keeps_the_fewest_lines() {
        let left = [Row::flow("aaaa bbbb cccc dddd eeee ffff")];
        let right = [Row::flow("gggg hhhh iiii jjjj")];
        let (right_width, height) = split(50, &left, &right);
        assert_eq!(height, 4, "{right_width}");
    }
}
