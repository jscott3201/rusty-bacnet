//! The Network screen's BDT consistency matrix: the grid of which BBMD
//! lists which, its legend, the findings, and the entries the survey did
//! not follow (#2130), each with its reason. The grid shows at most
//! [`MAX_SHOWN`] BBMDs and counts the rest, those past the matrix's own
//! limit ([`GRID_CAP`]) included.

use std::collections::HashSet;
use std::net::SocketAddrV4;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use bacnet_ops::bbmd::matrix::GRID_CAP;
use bacnet_ops::bbmd::{Cell, Finding, Matrix, NotFollowed, Unfollowed};

use super::{ADDRESS, note};
use crate::tui::app::App;
use crate::tui::view::Theme;

/// A cell as text: `uni` and `dir` for the two forwarding methods, with
/// `*` when the tables disagree on the mask.
fn cell_text(cell: Cell) -> String {
    match cell {
        Cell::Listed { mask, mixed } => {
            let method = if mask == std::net::Ipv4Addr::BROADCAST {
                "uni"
            } else {
                "dir"
            };
            format!("{method}{}", if mixed { "*" } else { "" })
        }
        Cell::Absent => "-".into(),
        Cell::Missing => "MISS".into(),
        Cell::Unknown => "?".into(),
    }
}

/// The matrix view, or how to start a survey before the first.
///
/// Below the grid, what fits goes by one rule (#2130): the findings first,
/// with a count of those cut; then the line on BBMDs past the grid's limit;
/// then the entries not followed, as many as fit and a count of the rest;
/// then the legend, which the help screen also carries. They are drawn grid, legend,
/// findings, limit, entries.
pub(super) fn matrix(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let state = &app.network;
    let Some((matrix, _)) = &state.matrix else {
        return note(
            frame,
            area,
            "Press m to read the BDT of every BBMD the target's table leads to.".into(),
            theme,
        );
    };
    let rows = grid_rows(matrix, &state.not_followed);
    let n = rows.len();
    let skipped = state.not_followed.len();
    // Entries not followed are no BBMDs: the list below names them, so
    // they take no row or column. Those past the grid's limit are in
    // `omitted` too, and are not counted as BBMDs left out.
    let in_grid = matrix.bbmds.len() - n;
    let omitted = matrix
        .omitted
        .saturating_sub(skipped.saturating_sub(in_grid));
    let shown = shown(n + omitted, area.width).min(n);
    let hidden = n - shown + omitted;
    let grid_h = shown as u16 + 1 + u16::from(hidden > 0);
    let [grid, below] =
        Layout::vertical([Constraint::Length(grid_h), Constraint::Min(0)]).areas(area);
    grid_table(frame, grid, matrix, &rows[..shown], hidden, theme);

    let width = usize::from(area.width);
    let mut room = usize::from(below.height);
    // Findings first: those that fit whole, and a count of the rest.
    let findings = findings_lines(matrix, &rows, omitted, skipped, theme);
    let heights: Vec<usize> = findings.iter().map(|line| wrapped(line, width)).collect();
    let mut listed = findings;
    let mut findings_h: usize = heights.iter().sum();
    if findings_h > room {
        let mut used = 0;
        let keep = heights
            .iter()
            .take_while(|&&h| {
                used += h;
                used < room
            })
            .count();
        let cut = listed.len() - keep;
        listed.truncate(keep);
        findings_h = heights[..keep].iter().sum::<usize>();
        if room > findings_h {
            let plural = if cut == 1 { "" } else { "s" };
            listed.push(Line::styled(
                format!("+{cut} more finding{plural}"),
                theme.warn(),
            ));
            findings_h += 1;
        }
    }
    room -= findings_h.min(room);
    // Then the BBMDs past the grid's limit.
    let limit = (omitted > 0).then(|| omitted_line(omitted, theme));
    let limit_h = limit.as_ref().map_or(0, |line| wrapped(line, width));
    let limit = limit.filter(|_| limit_h <= room);
    let limit_h = if limit.is_some() { limit_h } else { 0 };
    room -= limit_h;
    // Then the entries not followed, in the rows left.
    let notes = not_followed_lines(&state.not_followed, room, theme);
    room -= notes.len();
    // The legend last, whole or not at all.
    let mut legend_lines = vec![Line::raw("")];
    legend_lines.extend(
        legend(width)
            .into_iter()
            .map(|line| Line::styled(line, theme.dim())),
    );
    if legend_lines.len() > room {
        legend_lines.clear();
    }
    room -= legend_lines.len();
    // Spare rows go to the findings when nothing follows them, so a line
    // that wraps longer than counted still shows whole.
    if limit.is_none() && notes.is_empty() {
        findings_h += room;
    }
    let [legend_area, findings_area, limit_area, notes_area] = Layout::vertical([
        Constraint::Length(legend_lines.len() as u16),
        Constraint::Length(findings_h as u16),
        Constraint::Length(limit_h as u16),
        Constraint::Length(notes.len() as u16),
    ])
    .areas(below);
    frame.render_widget(Paragraph::new(legend_lines), legend_area);
    frame.render_widget(
        Paragraph::new(listed).wrap(Wrap { trim: true }),
        findings_area,
    );
    if let Some(limit) = limit {
        frame.render_widget(Paragraph::new(limit).wrap(Wrap { trim: true }), limit_area);
    }
    frame.render_widget(Paragraph::new(notes), notes_area);
}

/// The matrix's BBMDs that take a grid row and column: all but the
/// entries the survey did not follow.
fn grid_rows(matrix: &Matrix, not_followed: &[Unfollowed]) -> Vec<usize> {
    let skipped: HashSet<SocketAddrV4> = not_followed.iter().map(|s| s.address).collect();
    (0..matrix.bbmds.len())
        .filter(|&i| !skipped.contains(&matrix.bbmds[i]))
        .collect()
}

/// The findings list: the problems, the notes, or the line saying every
/// table read agrees. `omitted` counts BBMDs past the grid's limit, and
/// `skipped` the entries not followed.
fn findings_lines<'a>(
    matrix: &Matrix,
    rows: &[usize],
    omitted: usize,
    skipped: usize,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let notes_only = matrix
        .findings
        .iter()
        .all(|finding| matches!(finding, Finding::NoPeers(_)));
    // [`Matrix::consistent`] without the entries not followed, which no
    // read went to.
    let agree = omitted == 0
        && notes_only
        && rows
            .iter()
            .all(|&r| matrix.cells[r].iter().all(|cell| *cell != Cell::Unknown));
    let n = rows.len();
    if agree && !matrix.findings.is_empty() {
        // Only notes: an empty table lists nothing, itself included.
        matrix
            .findings
            .iter()
            .map(|finding| Line::styled(finding.describe(), theme.dim()))
            .collect()
    } else if agree && skipped > 0 {
        let others = match skipped {
            1 => "1 other entry was".to_string(),
            _ => format!("{skipped} other entries were"),
        };
        vec![Line::styled(
            format!(
                "All {n} BDTs read agree: each BBMD lists itself, every BBMD read is listed back, \
                 and masks match; {others} not read on purpose."
            ),
            theme.accent(),
        )]
    } else if agree {
        vec![Line::styled(
            format!(
                "All {n} BDTs agree: each BBMD lists itself, every entry is listed back, and masks match."
            ),
            theme.accent(),
        )]
    } else if matrix.findings.is_empty() {
        vec![Line::styled("Some BDTs were not read.", theme.warn())]
    } else {
        matrix
            .findings
            .iter()
            .map(|finding| Line::styled(format!("! {}", finding.describe()), theme.warn()))
            .collect()
    }
}

/// How many BBMDs are past the grid's limit.
fn omitted_line<'a>(omitted: usize, theme: &Theme) -> Line<'a> {
    let (count, are) = match omitted {
        1 => ("1 more BBMD".to_string(), "is"),
        _ => (format!("{omitted} more BBMDs"), "are"),
    };
    Line::styled(
        format!("{count} past the grid's limit of {GRID_CAP} {are} not shown."),
        theme.dim(),
    )
}

/// Rows `line` takes when wrapped between words to `width` columns.
fn wrapped(line: &Line, width: usize) -> usize {
    let width = width.max(1);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut rows = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        let need = if used == 0 { len } else { used + 1 + len };
        if need <= width {
            used = need;
        } else {
            rows += 1 + len.saturating_sub(1) / width;
            used = (len - 1) % width + 1;
        }
    }
    rows
}

/// A reason a BDT entry was not followed, in plain words.
fn not_followed_reason(why: NotFollowed) -> &'static str {
    match why {
        NotFollowed::Unresolved => "unresolved",
        NotFollowed::Unspecified => "unspecified",
        NotFollowed::Broadcast => "broadcast",
        NotFollowed::DirectedBroadcast => "directed broadcast",
        NotFollowed::Multicast => "multicast",
        NotFollowed::Own => "our own address",
        NotFollowed::Loopback => "loopback",
    }
}

/// The entries the survey did not follow in at most `room` lines: under a
/// heading, one line each, with a count of those that don't fit; with
/// under three lines, one line counting them all. They are dim, not
/// flagged: the survey left them out on purpose.
fn not_followed_lines<'a>(skipped: &[Unfollowed], room: usize, theme: &Theme) -> Vec<Line<'a>> {
    if skipped.is_empty() || room == 0 {
        return Vec::new();
    }
    if room < 3 && 1 + skipped.len() > room {
        let count = match skipped.len() {
            1 => "1 entry".to_string(),
            k => format!("{k} entries"),
        };
        return vec![Line::styled(
            format!("{count} not read on purpose (no read goes to such an address)"),
            theme.dim(),
        )];
    }
    let fit = if 1 + skipped.len() > room {
        room - 2
    } else {
        skipped.len()
    };
    let mut lines = vec![Line::styled(
        "Not read on purpose (no read goes to such an address):",
        theme.dim(),
    )];
    // Typed addresses only, but sanitized as every table's text is.
    lines.extend(skipped.iter().take(fit).map(|entry| {
        let text = format!(
            "{:<width$}  listed by {:<width$}  {}",
            entry.address.to_string(),
            entry.listed_by.to_string(),
            not_followed_reason(entry.why),
            width = usize::from(ADDRESS),
        );
        Line::styled(crate::tui::view::sanitize(&text), theme.dim())
    }));
    if fit < skipped.len() {
        lines.push(Line::styled(
            format!("+{} more not read on purpose", skipped.len() - fit),
            theme.dim(),
        ));
    }
    lines
}

/// The matrix legend's entries, packed into lines by [`legend`].
const LEGEND: [&str; 6] = [
    "uni = all-ones mask, sent to the BBMD",
    "dir = directed broadcast",
    "* = masks differ",
    "MISS = should be listed",
    "- = not listed",
    "? = not read",
];

/// The legend as lines at most `width` columns wide, breaking only between
/// entries.
fn legend(width: usize) -> Vec<String> {
    // One column spare for the `;` that ends a broken line.
    let room = width.saturating_sub(1);
    let mut lines = Vec::new();
    let mut line = "Row lists column:".to_string();
    for (i, item) in LEGEND.iter().enumerate() {
        let sep = if i == 0 { " " } else { "; " };
        if line.chars().count() + sep.len() + item.chars().count() > room {
            if i > 0 {
                line.push(';');
            }
            lines.push(std::mem::take(&mut line));
            line.push_str(item);
        } else {
            line.push_str(sep);
            line.push_str(item);
        }
    }
    lines.push(line);
    lines
}

/// Width of a grid cell, and of the `+k more` column.
const CELL: u16 = 5;
const MORE: u16 = 8;
/// The most BBMDs the grid shows; the findings below still name them all.
const MAX_SHOWN: usize = 16;

/// How many of `n` BBMDs fit a grid `width` columns wide, as rows and as
/// columns, leaving room for a `+k more` column when some don't.
fn shown(n: usize, width: u16) -> usize {
    let first = ADDRESS + 4;
    let fit = |extra: u16| usize::from(width.saturating_sub(first + extra) / (CELL + 1));
    if n <= fit(0).min(MAX_SHOWN) {
        n
    } else {
        fit(MORE + 1).min(MAX_SHOWN)
    }
}

/// The grid of the BBMDs at `rows` (indexes into the matrix), with a
/// `+k more` column for the `hidden` BBMDs that don't fit.
fn grid_table(
    frame: &mut Frame,
    area: Rect,
    matrix: &Matrix,
    rows: &[usize],
    hidden: usize,
    theme: &Theme,
) {
    let shown = rows.len();
    let more = || Span::styled(format!("+{hidden} more"), theme.warn());
    let mut header = vec![Span::raw("BBMD (row lists column)")];
    header.extend((1..=shown).map(|i| Span::raw(format!("#{i}"))));
    if hidden > 0 {
        header.push(more());
    }
    let mut lines: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, &r)| {
            let mut row = vec![Span::raw(format!("#{} {}", i + 1, matrix.bbmds[r]))];
            row.extend(rows.iter().map(|&c| {
                let cell = matrix.cells[r][c];
                let style = match cell {
                    Cell::Missing | Cell::Listed { mixed: true, .. } => theme.danger(),
                    Cell::Unknown => theme.warn(),
                    Cell::Absent => theme.dim(),
                    Cell::Listed { .. } => theme.accent(),
                };
                Span::styled(cell_text(cell), style)
            }));
            Row::new(row)
        })
        .collect();
    if hidden > 0 {
        lines.push(Row::new([more()]));
    }
    let mut widths = vec![Constraint::Length(ADDRESS + 4)];
    widths.extend(std::iter::repeat_n(Constraint::Length(CELL), shown));
    if hidden > 0 {
        widths.push(Constraint::Length(MORE));
    }
    let table = Table::new(lines, widths)
        .header(Row::new(header).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_read_as_text() {
        let unicast = std::net::Ipv4Addr::BROADCAST;
        let subnet = std::net::Ipv4Addr::new(255, 255, 255, 0);
        let listed = |mask, mixed| Cell::Listed { mask, mixed };
        assert_eq!(cell_text(listed(unicast, false)), "uni");
        assert_eq!(cell_text(listed(subnet, true)), "dir*");
        assert_eq!(cell_text(Cell::Missing), "MISS");
        assert_eq!(cell_text(Cell::Unknown), "?");
    }

    #[test]
    fn the_legend_breaks_between_entries_within_the_width() {
        for width in [40, 78, 118, 198] {
            let lines = legend(width);
            assert!(
                lines.iter().all(|line| line.chars().count() <= width),
                "{width}: {lines:?}"
            );
            let text = lines.join(" ");
            assert!(LEGEND.iter().all(|item| text.contains(item)), "{text}");
        }
        assert_eq!(legend(118).len(), 2);
        assert_eq!(legend(198).len(), 1);
    }

    #[test]
    fn the_grid_shows_what_fits_and_counts_the_rest() {
        // 118 columns inside the border: all of 12, 14 of 20.
        assert_eq!(shown(12, 118), 12);
        assert_eq!(shown(15, 118), 15);
        assert_eq!(shown(20, 118), 14);
        assert_eq!(shown(40, 400), MAX_SHOWN);
        assert_eq!(shown(3, 78), 3);
    }

    #[test]
    fn wrapping_counts_rows_between_words() {
        let rows = |text: &'static str, width| wrapped(&Line::raw(text), width);
        assert_eq!(rows("", 10), 1);
        assert_eq!(rows("one two", 7), 1);
        assert_eq!(rows("one two", 6), 2);
        // A word longer than the width takes the rows it fills.
        assert_eq!(rows("a abcdefghijkl", 5), 4);
    }

    #[test]
    fn entries_not_followed_fit_their_rows_and_count_the_rest() {
        let theme = Theme { color: false };
        let entry = |last| Unfollowed {
            address: SocketAddrV4::new(std::net::Ipv4Addr::new(224, 0, 0, last), 47808),
            listed_by: SocketAddrV4::new(std::net::Ipv4Addr::new(10, 0, 1, 1), 47808),
            why: NotFollowed::Multicast,
        };
        let skipped: Vec<Unfollowed> = (1..=4).map(entry).collect();
        let text = |room| -> Vec<String> {
            not_followed_lines(&skipped, room, &theme)
                .iter()
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        let full = text(9);
        assert_eq!(full.len(), 5);
        assert!(full[4].ends_with("multicast"), "{full:?}");
        let three = text(3);
        assert_eq!(three.len(), 3);
        assert_eq!(three[2], "+3 more not read on purpose");
        let one = ["4 entries not read on purpose (no read goes to such an address)"];
        assert_eq!(text(2), one);
        assert_eq!(text(1), one);
        assert!(text(0).is_empty());
        assert!(not_followed_lines(&[], 5, &theme).is_empty());
    }
}
