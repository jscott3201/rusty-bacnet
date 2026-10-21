//! The Network screen: the foreign-device panel on top, and below it the
//! target BBMD's BDT or FDT, the consistency matrix, or the routers heard
//! and the networks they reach (Tab switches).
//! Every state is marked by text as well as colour.
//!
//! From [`WIDE_WIDTH`] columns, once the whole BDT fits beside the FDT, the
//! two sit side by side, with the matrix beneath them when the height
//! allows; Tab then moves the focus (the accented pane) instead of switching
//! what is shown. A table longer than its pane counts the rows it hides.

use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, LineGauge, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use bacnet_transport::bip::ForeignRegistrationOutcome;

use super::{Theme, WIDE_WIDTH};
use crate::tui::app::devices::age_label;
use crate::tui::app::network::{Fetched, NetworkState, View};
use crate::tui::app::objects::kebab;
use crate::tui::app::App;
use crate::tui::network::matrix::{Cell, Matrix};

/// Height of the foreign-device panel, borders included, with the
/// registration counters on the status line.
const PANEL_HEIGHT: u16 = 7;

/// Width of the BBMD address column: `255.255.255.255:65535`.
const ADDRESS: u16 = 21;

/// Widths of the FDT's TTL and Left columns, and of its marker column.
const FDT_TTL: u16 = 8;
const FDT_MARK: u16 = 11;
/// Width of the FDT table, column gaps included.
const FDT_WIDTH: u16 = ADDRESS + 2 * FDT_TTL + FDT_MARK + 3 * 2;

/// Rows the wide layout needs inside its border to fit the matrix beneath
/// the BDT and FDT.
const WIDE_MATRIX_ROWS: u16 = 20;

/// The Network screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let [panel, tables] = Layout::vertical([
        Constraint::Length(panel_height(app, area.width)),
        Constraint::Min(3),
    ])
    .areas(area);
    foreign_panel(frame, panel, app, theme);
    let state = &app.network;
    let tabs = [
        (View::Bdt, "BDT"),
        (View::Fdt, "FDT"),
        (View::Matrix, "Matrix"),
        (View::Networks, "Networks"),
    ];
    let mut title = vec![Span::styled(" ", theme.accent())];
    for (view, name) in tabs {
        let text = if view == state.view {
            format!("[{name}]")
        } else {
            name.to_string()
        };
        let style = if view == state.view {
            theme.accent()
        } else {
            theme.dim()
        };
        title.push(Span::styled(text, style));
        title.push(Span::raw(" "));
    }
    title.push(Span::styled(subtitle(app), theme.dim()));
    let block = Block::bordered()
        .title(Line::from(title))
        .border_style(theme.dim());
    let inner = block.inner(tables);
    frame.render_widget(block, tables);
    if state.view == View::Networks {
        super::networks::draw(frame, inner, app, theme);
        return;
    }
    if app.transport != "BIP" {
        let text = "B/IP only: BBMDs and foreign devices belong to BACnet/IP (Annex J).";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), inner);
        return;
    }
    if wide(app, area) {
        return side_by_side(frame, inner, app, theme);
    }
    match state.view {
        View::Bdt => bdt(frame, inner, app, theme),
        View::Fdt => fdt(frame, inner, app, theme),
        View::Matrix => matrix(frame, inner, app, theme),
        View::Networks => {}
    }
}

/// Which BBMD the tables are from, and how fresh they are.
fn subtitle(app: &App) -> String {
    let state = &app.network;
    if state.view == View::Networks {
        return super::networks::subtitle(app);
    }
    let busy = match state.view {
        View::Matrix if state.surveying => Some("surveying"),
        View::Bdt | View::Fdt if state.reading => Some("reading"),
        _ => None,
    };
    let at = match state.view {
        View::Bdt => state.bdt.as_ref().map(|f| f.at),
        View::Fdt => state.fdt.as_ref().map(|f| f.at),
        View::Matrix => state.matrix.as_ref().map(|(_, at)| *at),
        View::Networks => None,
    };
    let target = match state.target {
        None => "no BBMD".to_string(),
        Some(t) if state.foreign.is_some() && !state.tables_are_ours() => {
            format!("{t}, not our BBMD")
        }
        Some(t) => t.to_string(),
    };
    match (busy, at) {
        (Some(busy), _) => format!("| {target} | {busy} "),
        (None, Some(at)) => format!("| {target} | read {} ago ", age_label(app.now, at)),
        (None, None) => format!("| {target} | not read: r reads, m surveys "),
    }
}

fn seconds(duration: Duration) -> u64 {
    // A countdown shows the second it is in, so 0.4 s left reads 1 s.
    duration.as_millis().div_ceil(1_000) as u64
}

fn foreign_panel(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" Foreign device ", theme.accent()))
        .border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let state = &app.network;
    let Some(foreign) = state.foreign else {
        let lines = vec![
            Line::styled("Not registered as a foreign device.", theme.dim()),
            Line::styled(
                "Connect with --foreign-bbmd ADDRESS to register; :bbmd ADDRESS reads any BBMD's tables.",
                theme.dim(),
            ),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        return;
    };
    let (word, style, counts) = status(app, theme);
    let inline = counters_inline(&word, &counts, inner.width);
    // The counters take their own line when they don't fit the status's.
    let lines = 5 + usize::from(!inline);
    let areas = Layout::vertical(vec![Constraint::Length(1); lines]).split(inner);
    let [head, status_area] = [areas[0], areas[1]];
    let [result, gauge, own] = [areas[lines - 3], areas[lines - 2], areas[lines - 1]];
    let line = Line::from(vec![
        Span::styled("BBMD ", theme.dim()),
        Span::styled(foreign.bbmd.to_string(), theme.accent()),
        Span::styled(format!("   TTL {} s", foreign.ttl), theme.dim()),
        Span::styled(
            format!("   renews every {} s", seconds(foreign.renewal)),
            theme.dim(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), head);

    let snapshot = state.registration.as_ref().map(|(s, _)| *s);
    let refused =
        snapshot.and_then(|s| s.last_outcome) == Some(ForeignRegistrationOutcome::Refused);
    let mut line = vec![
        Span::styled("Status ", theme.dim()),
        Span::styled(word, style),
    ];
    if inline && !counts.is_empty() {
        line.push(Span::styled(format!("   {counts}"), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(line)), status_area);
    if !inline {
        let line = Line::from(vec![
            Span::styled("Counts ", theme.dim()),
            Span::styled(counts, theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(line), areas[2]);
    }

    let last = match snapshot.and_then(|s| s.last_result) {
        Some(code) => format!("{} (0x{:04X})", kebab(code), code.to_raw()),
        None => "none yet".to_string(),
    };
    let outcome = snapshot
        .and_then(|s| s.last_outcome)
        .map_or_else(String::new, |o| format!(", attempt {}", outcome_label(o)));
    let line = Line::from(vec![
        Span::styled("Last BVLC-Result ", theme.dim()),
        Span::styled(
            last,
            if refused {
                theme.danger()
            } else {
                theme.accent()
            },
        ),
        Span::styled(outcome, theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(line), result);

    match state.next_attempt(app.now) {
        Some(left) => {
            let total = foreign.renewal.as_secs_f64().max(0.001);
            let ratio = (left.as_secs_f64() / total).clamp(0.0, 1.0);
            let widget = LineGauge::default()
                .ratio(ratio)
                .label(format!("Next registration in {:>3} s ", seconds(left)))
                .filled_symbol("█")
                .unfilled_symbol("░")
                .filled_style(theme.accent())
                .unfilled_style(theme.dim());
            frame.render_widget(widget, gauge);
        }
        None => frame.render_widget(
            Paragraph::new(Line::styled(
                "Next registration: not scheduled",
                theme.dim(),
            )),
            gauge,
        ),
    }
    frame.render_widget(Paragraph::new(own_line(state, app, theme, own.width)), own);
}

/// The registration status word and its style, and the counters (empty
/// before the first snapshot).
fn status(app: &App, theme: &Theme) -> (String, Style, String) {
    let state = &app.network;
    let snapshot = state.registration.as_ref().map(|(s, _)| *s);
    let refused =
        snapshot.and_then(|s| s.last_outcome) == Some(ForeignRegistrationOutcome::Refused);
    let (word, style) = match (snapshot, state.lease_left(app.now)) {
        (_, _) if refused => ("REFUSED".to_string(), theme.danger()),
        (Some(s), Some(left)) if s.registered => (
            format!("registered, lease {} s left", seconds(left)),
            theme.accent(),
        ),
        (Some(_), _) => ("not registered".to_string(), theme.warn()),
        (None, _) => ("waiting for the first answer".to_string(), theme.dim()),
    };
    let counts = snapshot.map_or_else(String::new, |s| {
        format!(
            "attempts {}  accepted {}  refused {}  lapses {}",
            s.attempts, s.accepted, s.refused, s.lapses
        )
    });
    (word, style, counts)
}

/// Whether the counters fit on the status line, `width` columns wide.
fn counters_inline(word: &str, counts: &str, width: u16) -> bool {
    let need = "Status ".len() + word.chars().count() + 3 + counts.chars().count();
    counts.is_empty() || need <= usize::from(width)
}

/// Height of the foreign-device panel at `width`, borders included: one
/// more line when the counters don't fit beside the status.
fn panel_height(app: &App, width: u16) -> u16 {
    if app.network.foreign.is_none() {
        return PANEL_HEIGHT;
    }
    // Only the text's width matters here, not its colour.
    let (word, _, counts) = status(app, &Theme { color: false });
    PANEL_HEIGHT + u16::from(!counters_inline(&word, &counts, width.saturating_sub(2)))
}

fn outcome_label(outcome: ForeignRegistrationOutcome) -> &'static str {
    match outcome {
        ForeignRegistrationOutcome::Pending => "pending",
        ForeignRegistrationOutcome::Accepted => "accepted",
        ForeignRegistrationOutcome::Refused => "refused",
        ForeignRegistrationOutcome::TimedOut => "unanswered",
        ForeignRegistrationOutcome::LocalError => "not sent",
        ForeignRegistrationOutcome::Cancelled => "cancelled",
    }
}

/// What the BBMD recorded for us, from the last FDT read. Only the BBMD we
/// register with is expected to hold our entry: after `:bbmd` the line says
/// whose tables are shown instead.
fn own_line<'a>(state: &NetworkState, app: &App, theme: &Theme, width: u16) -> Line<'a> {
    if let (Some(target), false) = (state.target, state.tables_are_ours()) {
        let mut text = format!("Our FDT entry not checked: tables are from {target}, not our BBMD");
        // The way back, when the line has room for it.
        if let Some(ours) = state.foreign.map(|f| f.bbmd) {
            let back = format!("; :bbmd {ours} for ours");
            if text.chars().count() + back.chars().count() <= usize::from(width) {
                text.push_str(&back);
            }
        }
        return Line::styled(text, theme.dim());
    }
    let label = Span::styled("As the BBMD sees us ", theme.dim());
    let Some(Fetched {
        result: Ok(entries),
        ..
    }) = &state.fdt
    else {
        return Line::from(vec![label, Span::styled("FDT not read yet", theme.dim())]);
    };
    match state.own.and_then(|i| entries.get(i)) {
        Some(entry) => Line::from(vec![
            label,
            Span::styled(entry.address.to_string(), theme.selected()),
            Span::styled(
                // The BBMD keeps an FDT entry 30 s past its TTL (Annex J).
                format!(
                    "  TTL {} s + 30 s grace, {} s left",
                    entry.ttl,
                    state.remaining(entry, app.now)
                ),
                theme.dim(),
            ),
        ]),
        None => Line::from(vec![
            label,
            Span::styled(
                format!(
                    "no FDT entry has our address {}; a NAT router may have rewritten it",
                    state.local.map_or_else(String::new, |l| l.to_string())
                ),
                theme.warn(),
            ),
        ]),
    }
}

/// An error or an empty table as one line.
fn note(frame: &mut Frame, area: Rect, text: String, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Line::styled(text, theme.warn())).wrap(Wrap { trim: true }),
        area,
    );
}

/// Width of the BDT's Mask column.
const MASK: u16 = 15;
/// The BDT's header; its last column is also its narrowest content.
const BDT_HEADER: [&str; 3] = ["BBMD", "Mask", "How peers forward to it (Annex J.4.5)"];

/// The BDT's rows as text: address, mask and how peers forward to it.
fn bdt_rows(app: &App) -> Vec<[String; 3]> {
    let Some(Fetched {
        bbmd,
        result: Ok(entries),
        ..
    }) = &app.network.bdt
    else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|entry| {
            let mut forwarding = entry.explain();
            if entry.address == *bbmd {
                forwarding = format!("(this BBMD) {forwarding}");
            }
            [
                entry.address.to_string(),
                entry.mask.to_string(),
                forwarding,
            ]
        })
        .collect()
}

/// Width the BDT needs to show every row whole.
fn bdt_width(app: &App) -> u16 {
    let forwarding = bdt_rows(app)
        .iter()
        .map(|[_, _, f]| f.chars().count())
        .chain([BDT_HEADER[2].len()])
        .max()
        .unwrap_or(0);
    ADDRESS + MASK + 2 * 2 + u16::try_from(forwarding).unwrap_or(u16::MAX)
}

/// The rows that fit `height` (a header line included): when some don't,
/// the last line says how many are hidden instead.
fn fit_rows<'a>(mut rows: Vec<Row<'a>>, height: u16, theme: &Theme) -> Vec<Row<'a>> {
    let room = usize::from(height.saturating_sub(1));
    if rows.len() > room {
        let shown = room.saturating_sub(1);
        let hidden = rows.len() - shown;
        rows.truncate(shown);
        rows.push(Row::new([format!("+{hidden} more")]).style(theme.warn()));
    }
    rows
}

fn bdt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(read) = &app.network.bdt else { return };
    match &read.result {
        Ok(entries) if entries.is_empty() => {
            return note(
                frame,
                area,
                format!("{} has an empty BDT.", read.bbmd),
                theme,
            )
        }
        Ok(_) => {}
        Err(error) => return note(frame, area, format!("BDT of {}: {error}", read.bbmd), theme),
    }
    let rows: Vec<Row> = bdt_rows(app).into_iter().map(Row::new).collect();
    let widths = [
        Constraint::Length(ADDRESS),
        Constraint::Length(MASK),
        Constraint::Min(10),
    ];
    let table = Table::new(fit_rows(rows, area.height, theme), widths)
        .header(Row::new(BDT_HEADER).style(theme.accent()))
        .column_spacing(2);
    frame.render_widget(table, area);
}

fn fdt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let state = &app.network;
    let Some(read) = &state.fdt else { return };
    let entries = match &read.result {
        Ok(entries) if entries.is_empty() => {
            return note(
                frame,
                area,
                format!("{} has no foreign devices.", read.bbmd),
                theme,
            )
        }
        Ok(entries) => entries,
        Err(error) => return note(frame, area, format!("FDT of {}: {error}", read.bbmd), theme),
    };
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let ours = state.own == Some(i);
            let row = Row::new([
                entry.address.to_string(),
                format!("{} s", entry.ttl),
                format!("{} s", state.remaining(entry, app.now)),
                if ours {
                    "this client".into()
                } else {
                    String::new()
                },
            ]);
            if ours {
                row.style(theme.selected())
            } else {
                row
            }
        })
        .collect();
    let widths = [
        Constraint::Length(ADDRESS),
        Constraint::Length(FDT_TTL),
        Constraint::Length(FDT_TTL),
        Constraint::Length(FDT_MARK),
    ];
    let table = Table::new(fit_rows(rows, area.height, theme), widths)
        .header(Row::new(["Foreign device", "TTL", "Left", ""]).style(theme.accent()))
        .column_spacing(2);
    frame.render_widget(table, area);
}

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

fn matrix(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some((matrix, _)) = &app.network.matrix else {
        return note(
            frame,
            area,
            "Press m to read the BDT of every BBMD the target's table leads to.".into(),
            theme,
        );
    };
    let n = matrix.bbmds.len();
    let shown = shown(n, area.width);
    let findings = matrix.findings.len().max(1);
    let mut legend_lines = vec![Line::raw("")];
    legend_lines.extend(
        legend(usize::from(area.width))
            .into_iter()
            .map(|line| Line::styled(line, theme.dim())),
    );
    let [grid, legend_area, list] = Layout::vertical([
        Constraint::Length(shown as u16 + 1 + u16::from(shown < n)),
        Constraint::Length(legend_lines.len() as u16),
        Constraint::Min(findings.min(8) as u16),
    ])
    .areas(area);
    grid_table(frame, grid, matrix, theme);
    frame.render_widget(Paragraph::new(legend_lines), legend_area);
    let lines: Vec<Line> = if matrix.consistent() {
        vec![Line::styled(
            format!("All {n} BDTs agree: each BBMD lists itself, every entry is listed back, and masks match."),
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
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), list);
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

/// Whether the screen, drawn in `body`, uses the wide layout: from
/// [`WIDE_WIDTH`] columns, when the whole BDT fits beside the FDT, on the
/// BDT and FDT views, and on the matrix when it also fits beneath them.
/// Tab then moves the focus between the panes.
pub(super) fn wide(app: &App, body: Rect) -> bool {
    if app.transport != "BIP" || body.width < WIDE_WIDTH {
        return false;
    }
    // Inside the tables' border, below the foreign-device panel.
    let width = body.width.saturating_sub(2);
    let height = body
        .height
        .saturating_sub(panel_height(app, body.width) + 2);
    let fits = bdt_width(app) + 2 + FDT_WIDTH + 2 <= width;
    fits && match app.network.view {
        View::Bdt | View::Fdt => true,
        View::Matrix => height >= WIDE_MATRIX_ROWS,
        View::Networks => false,
    }
}

/// The wide layout: the BDT and FDT side by side, the matrix beneath them
/// when the height allows. The selected table's pane is accented.
fn side_by_side(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let state = &app.network;
    let (top, bottom) = if area.height >= WIDE_MATRIX_ROWS {
        let rows = |len: Option<usize>| len.unwrap_or(1).max(1) as u16;
        let bdt_rows = rows(
            state
                .bdt
                .as_ref()
                .and_then(|f| f.result.as_ref().ok().map(Vec::len)),
        );
        let fdt_rows = rows(
            state
                .fdt
                .as_ref()
                .and_then(|f| f.result.as_ref().ok().map(Vec::len)),
        );
        // The longer table, its header and the borders, at most half; a
        // longer table says how many rows it hides.
        let height = (bdt_rows.max(fdt_rows) + 3).min(area.height / 2);
        let [top, bottom] =
            Layout::vertical([Constraint::Length(height), Constraint::Min(3)]).areas(area);
        (top, Some(bottom))
    } else {
        (area, None)
    };
    // The FDT takes only its own width, and the BDT, whose text is longer,
    // the rest.
    let [left, right] = Layout::horizontal([
        Constraint::Min(bdt_width(app) + 2),
        Constraint::Length(FDT_WIDTH + 2),
    ])
    .areas(top);
    let pane = |frame: &mut Frame, view: View, name: &str, area: Rect| {
        let style = if view == state.view {
            theme.accent()
        } else {
            theme.dim()
        };
        let block = Block::bordered()
            .title(Line::styled(format!(" {name} "), style))
            .border_style(style);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    };
    let inner = pane(frame, View::Bdt, "BDT", left);
    bdt(frame, inner, app, theme);
    let inner = pane(frame, View::Fdt, "FDT", right);
    fdt(frame, inner, app, theme);
    if let Some(bottom) = bottom {
        let inner = pane(frame, View::Matrix, "Matrix", bottom);
        matrix(frame, inner, app, theme);
    }
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

fn grid_table(frame: &mut Frame, area: Rect, matrix: &Matrix, theme: &Theme) {
    let n = matrix.bbmds.len();
    let shown = shown(n, area.width);
    let hidden = n - shown;
    let more = || Span::styled(format!("+{hidden} more"), theme.warn());
    let mut header = vec![Span::raw("BBMD (row lists column)")];
    header.extend((1..=shown).map(|i| Span::raw(format!("#{i}"))));
    if hidden > 0 {
        header.push(more());
    }
    let mut rows: Vec<Row> = matrix
        .bbmds
        .iter()
        .zip(&matrix.cells)
        .take(shown)
        .enumerate()
        .map(|(i, (bbmd, cells))| {
            let mut row = vec![Span::raw(format!("#{} {bbmd}", i + 1))];
            row.extend(cells.iter().take(shown).map(|cell| {
                let style = match cell {
                    Cell::Missing | Cell::Listed { mixed: true, .. } => theme.danger(),
                    Cell::Unknown => theme.warn(),
                    Cell::Absent => theme.dim(),
                    Cell::Listed { .. } => theme.accent(),
                };
                Span::styled(cell_text(*cell), style)
            }));
            Row::new(row)
        })
        .collect();
    if hidden > 0 {
        rows.push(Row::new([more()]));
    }
    let mut widths = vec![Constraint::Length(ADDRESS + 4)];
    widths.extend(std::iter::repeat_n(Constraint::Length(CELL), shown));
    widths.push(Constraint::Length(MORE));
    let table = Table::new(rows, widths)
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
        assert_eq!(seconds(Duration::from_millis(400)), 1);
        assert_eq!(seconds(Duration::ZERO), 0);
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
}
