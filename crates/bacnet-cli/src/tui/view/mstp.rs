//! The MS/TP screen (#969): this station and its place in the token ring on
//! the left, the transport's counters on the right; and the join
//! confirmation. Every state is marked by text as well as colour.
//!
//! Each pane is as wide as its content where both fit (#1893), and a pane
//! too short for its rows ends with `+N more`. The keys are in the footer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};

use super::{Theme, centered, more, text_width};
use crate::tui::app::App;
use crate::tui::app::mstp::{
    JoinDialog, JoinOrigin, LATENESS_BANDS, Lateness, LatenessMax, MstpState,
};

/// The MS/TP screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mstp = &app.mstp;
    if let (None, Some(monitor)) = (&mstp.station, app.monitor.as_deref()) {
        return super::monitor::draw(frame, area, monitor, theme);
    }
    if mstp.station.is_none() {
        let block = Block::bordered()
            .title(Line::styled(" MS/TP ", theme.accent()))
            .border_style(theme.dim());
        let text = if app.transport == "MSTP" {
            "Not on the trunk yet: confirm joining it (y), or connect again (c)."
        } else {
            "MS/TP only: start bacnet tui with --mstp PORT --mstp-mac N, or connect an \
             MS/TP profile (p), to join a trunk and see this station's token state here; \
             or with --mstp-monitor PORT to listen to one without transmitting."
        };
        let lines = Paragraph::new(Line::styled(text, theme.dim())).wrap(Wrap { trim: true });
        frame.render_widget(lines.block(block), area);
        return;
    }
    let station = participant_lines(mstp, theme);
    let (counters_title, counters) = counter_lines(mstp, area.height, theme);
    // Each pane's content, its borders and padding.
    let fitted = |lines: &[Line], title: &Line| {
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        (widest + 4).max(title.width() + 2) as u16
    };
    let title = Line::raw(STATION_TITLE);
    let (left, right) = (fitted(&station, &title), fitted(&counters, &counters_title));
    let widths = if left + right <= area.width {
        [Constraint::Length(left), Constraint::Length(right)]
    } else {
        [Constraint::Percentage(45), Constraint::Percentage(55)]
    };
    let [left, right] = Layout::horizontal(widths).areas(area);
    let block = Block::bordered()
        .title(Line::styled(STATION_TITLE, theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let paragraph = Paragraph::new(station).wrap(Wrap { trim: false });
    frame.render_widget(paragraph.block(block), left);
    let block = Block::bordered()
        .title(counters_title)
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let counters = more(counters, right.height.saturating_sub(2), theme);
    frame.render_widget(Paragraph::new(counters).block(block), right);
}

/// The station pane's title.
const STATION_TITLE: &str = " This station ";

/// The station pane: the settings joined with and the token state.
fn participant_lines(mstp: &MstpState, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(station) = &mstp.station {
        lines.push(Line::raw(format!("Port         {}", station.port)));
        lines.push(Line::raw(format!("Station      mstp:{}", station.mac)));
        lines.push(Line::raw(format!("Baud         {}", station.baud)));
        lines.push(Line::raw(format!("Max master   {}", station.max_master)));
        lines.push(Line::raw(format!(
            "Info frames  {}",
            station.max_info_frames
        )));
    }
    if let Some(thread) = &mstp.thread {
        lines.push(Line::raw(format!("Thread       {thread}")));
    }
    lines.push(Line::raw(""));
    match mstp.shown.and_then(|sample| sample.node) {
        None => lines.push(Line::styled("Token        not reported yet", theme.dim())),
        Some(node) => {
            lines.push(if node.holds_token {
                Line::styled("Token        HELD", theme.accent())
            } else {
                Line::raw("Token        waiting")
            });
            lines.push(Line::raw(format!("State        {}", node.state)));
            lines.push(Line::raw(format!(
                "Next station mstp:{}",
                node.next_station
            )));
            lines.push(Line::raw(format!(
                "Polling      mstp:{}",
                node.poll_station
            )));
            if node.sole_master {
                lines.push(Line::styled(
                    "SOLE MASTER: no other master answers",
                    theme.warn(),
                ));
            } else {
                lines.push(Line::raw("Sole master  no"));
            }
        }
    }
    lines
}

/// The counters pane's title and rows, for a pane `height` rows tall:
/// the lateness bands only where they fit.
fn counter_lines(
    mstp: &MstpState,
    height: u16,
    theme: &Theme,
) -> (Line<'static>, Vec<Line<'static>>) {
    let since = if mstp.baseline.is_some() {
        "since clear"
    } else {
        "since joining"
    };
    let state = if mstp.paused { "PAUSED" } else { "live" };
    let title = Line::from(vec![
        Span::styled(" Counters ", theme.accent()),
        Span::styled(format!("({since}, {state}) "), theme.dim()),
    ]);
    let lines: Vec<Line> = match mstp.counters() {
        None => vec![Line::styled("No reading yet.", theme.dim())],
        Some(rows) => {
            let mut lines: Vec<Line> = rows
                .into_iter()
                .map(|(label, value)| Line::raw(row(&label, &value.to_string())))
                .collect();
            if let Some(lateness) = mstp.lateness() {
                // Inside the borders.
                let room = usize::from(height.saturating_sub(2));
                lines.extend(lateness_lines(&lateness, room.saturating_sub(lines.len())));
            }
            lines
        }
    };
    (title, lines)
}

/// A counter row: its label, then its value right-aligned.
fn row(label: &str, value: &str) -> String {
    let pad = 32usize.saturating_sub(text_width(label) + text_width(value));
    format!("{label}{:pad$}{value}", "")
}

/// The timer-lateness rows, in `room` rows: the largest lateness, then the
/// histogram's buckets when they fit too.
fn lateness_lines(lateness: &Lateness, room: usize) -> Vec<Line<'static>> {
    let max = match lateness.max {
        None => "-".to_string(),
        Some(LatenessMax::Exact(max)) => millis(max, false),
        Some(LatenessMax::AtMost(max)) => format!("≤{}", millis(max, true)),
        Some(LatenessMax::Below(edge)) => format!("<{} ms", edge.as_millis()),
    };
    let mut lines = vec![Line::raw(row("Timer lateness max", &max))];
    if room > LATENESS_BANDS.len() {
        lines.extend(
            LATENESS_BANDS
                .iter()
                .zip(lateness.buckets)
                .map(|(band, count)| {
                    Line::raw(row(&format!("  timers {band}"), &count.to_string()))
                }),
        );
    }
    lines
}

/// A lateness in milliseconds to a tenth, or whole ones from 100 ms: cut
/// down, or rounded `up` for an upper bound, which then never reads below
/// the value. A whole number of milliseconds under 100 keeps its tenth
/// (`2.0 ms`).
fn millis(duration: std::time::Duration, up: bool) -> String {
    let nanos = duration.as_nanos();
    let step = if nanos >= 100_000_000 {
        1_000_000
    } else {
        100_000
    };
    let steps = if up {
        nanos.div_ceil(step)
    } else {
        nanos / step
    };
    if step == 1_000_000 {
        format!("{steps} ms")
    } else {
        format!("{}.{} ms", steps / 10, steps % 10)
    }
}

/// The join confirmation (T2): what joining does, the port, then `y`.
pub(super) fn join(
    frame: &mut Frame,
    area: Rect,
    dialog: &JoinDialog,
    thread: &str,
    theme: &Theme,
) {
    let rows = dialog
        .ports
        .len()
        .min(usize::from(area.height.saturating_sub(14)).max(1));
    let note = match &dialog.origin {
        JoinOrigin::Monitor { note } => Some(note.clone()),
        _ => None,
    };
    let extra = u16::from(note.is_some());
    let box_area = centered(area, 76, rows as u16 + 10 + extra);
    frame.render_widget(Clear, box_area);
    let s = &dialog.settings;
    let mut lines = vec![
        Line::raw(format!(
            "Join as master mstp:{}: {} baud, Max_Master {}, Max_Info_Frames {}.",
            s.mac, s.baud, s.max_master, s.max_info_frames
        )),
        Line::raw(format!("Token-handling thread: {thread}.")),
        Line::styled(
            "This station will take part in token passing: every station on the trunk",
            theme.warn(),
        ),
        Line::styled(
            "sees it, and a MAC another device already uses disrupts the whole ring.",
            theme.warn(),
        ),
    ];
    lines.extend(note.map(Line::raw));
    lines.extend([Line::raw(""), Line::styled("Serial port:", theme.accent())]);
    let offset = dialog.selected.saturating_sub(rows - 1);
    for (i, port) in dialog.ports.iter().enumerate().skip(offset).take(rows) {
        let text = format!("{} {port}", if i == dialog.selected { ">" } else { " " });
        lines.push(if i == dialog.selected {
            Line::styled(text, theme.selected())
        } else {
            Line::raw(text)
        });
    }
    lines.push(Line::raw(""));
    let keys = if dialog.ports.len() > 1 {
        "y join | Up/Down port | Esc cancel"
    } else {
        "y join | Esc cancel"
    };
    lines.push(Line::styled(keys, theme.dim()));
    let block = Block::bordered()
        .title(Line::styled(" Join the MS/TP trunk? ", theme.warn()))
        .border_style(theme.warn())
        .padding(Padding::horizontal(1));
    frame.render_widget(Paragraph::new(lines).block(block), box_area);
}

#[cfg(test)]
mod tests {
    use super::millis;
    use std::time::Duration;

    /// Tenths under 100 ms, whole milliseconds from it; cut down for a
    /// value, rounded up for a bound.
    #[test]
    fn lateness_reads_in_tenths_then_whole_milliseconds() {
        let us = Duration::from_micros;
        for (value, down, up) in [
            (us(0), "0.0 ms", "0.0 ms"),
            (us(6_400), "6.4 ms", "6.4 ms"),
            (us(6_401), "6.4 ms", "6.5 ms"),
            (us(99_999), "99.9 ms", "100.0 ms"),
            (us(100_000), "100 ms", "100 ms"),
            (us(123_456), "123 ms", "124 ms"),
        ] {
            assert_eq!(millis(value, false), down, "{value:?}");
            assert_eq!(millis(value, true), up, "{value:?}");
        }
    }
}
