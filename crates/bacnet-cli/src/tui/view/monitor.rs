//! The MS/TP monitor screen (#970): a summary, the tab bar, and one of four
//! views: the station grid, the token ring, findings with the join wizard,
//! and error counters. Roles, warnings and the selected tab are marked by
//! text as well as colour.
//!
//! The summary wraps between its items where a line can't hold them all
//! (#1893). The station view stacks the address map over the station table,
//! and from [`WIDE_WIDTH`] columns, where both fit, puts them side by side.
//! A view that can't show all its rows ends with `+N more`.

use std::collections::VecDeque;
use std::time::Duration;

use bacnet_transport::mstp::monitor::{MaxMaster, MonitorStats};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};

use super::columns::{self, Column};
use super::watch::sparkline;
use super::{Theme, WIDE_WIDTH, ellipsize, more, text_width};
use crate::tui::app::monitor::{MonitorState, MonitorTab, Wizard};
use crate::tui::monitor::MonitorSample;
use crate::tui::monitor::analysis::{self, Role, Severity, millis};

/// The monitor screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, monitor: &MonitorState, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::styled(" MS/TP monitor: listen only ", theme.accent()))
        .border_style(theme.dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let sample = monitor.shown.as_deref();
    let summary = summary_lines(monitor, sample, usize::from(inner.width));
    let [summary_area, tabs, body] = Layout::vertical([
        Constraint::Length(summary.len() as u16),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(summary), summary_area);
    frame.render_widget(Paragraph::new(tab_bar(monitor, theme)), tabs);
    if monitor.tab == MonitorTab::Scan {
        return super::monitor_scan::draw(frame, body, monitor, theme);
    }
    let Some(sample) = sample else {
        let text = if monitor.live {
            "Listening; the first reading comes within a second."
        } else {
            "Monitor is not listening. See Scan for progress or restoration errors."
        };
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), body);
        return;
    };
    let lines = match monitor.tab {
        MonitorTab::Stations => return draw_stations(frame, body, sample, theme),
        MonitorTab::Ring => ring(monitor, sample, usize::from(body.width), theme),
        MonitorTab::Findings => findings(monitor, sample, theme),
        MonitorTab::Errors => errors(monitor, sample, theme),
        MonitorTab::Scan => unreachable!("scan rendered before sample"),
    };
    let paragraph = if matches!(monitor.tab, MonitorTab::Findings | MonitorTab::Errors) {
        Paragraph::new(lines).wrap(Wrap { trim: false })
    } else {
        Paragraph::new(more(lines, body.height, theme))
    };
    frame.render_widget(paragraph, body);
}

/// The summary's items, packed into lines `width` columns wide: an item
/// goes on the next line rather than being cut, and only an item wider
/// than a whole line is cut.
fn summary_lines(
    monitor: &MonitorState,
    sample: Option<&MonitorSample>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut items = vec![monitor.port.clone(), format!("{} baud", monitor.baud)];
    if let Some(sample) = sample {
        let stations = sample.stats.present_stations().count();
        items.extend([
            format!("up {} s", sample.elapsed.as_secs()),
            format!("{stations} stations"),
            format!("timestamps ±{}", millis(sample.stats.timing.resolution)),
        ]);
    }
    if monitor.paused {
        items.push("PAUSED".into());
    }
    let mut lines: Vec<String> = Vec::new();
    for item in items {
        let item = ellipsize(&item, width);
        match lines.last_mut() {
            Some(line) if text_width(line) + 2 + text_width(&item) <= width => {
                line.push_str("  ");
                line.push_str(&item);
            }
            _ => lines.push(item),
        }
    }
    lines.into_iter().map(Line::raw).collect()
}

fn tab_bar(monitor: &MonitorState, theme: &Theme) -> Line<'static> {
    let findings = monitor.latest.as_deref().map_or(0, |sample| {
        analysis::findings(sample, &monitor.thresholds).len()
    });
    let mut spans = Vec::new();
    for tab in MonitorTab::ALL {
        let mut title = tab.title().to_string();
        if tab == MonitorTab::Findings && findings > 0 {
            title.push_str(&format!(" ({findings})"));
        }
        spans.push(if tab == monitor.tab {
            Span::styled(format!("[{title}]"), theme.selected())
        } else {
            Span::raw(format!(" {title} "))
        });
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("Tab: next view", theme.dim()));
    Line::from(spans)
}

/// The address map's width with 64 addresses a row: a label, a range and
/// the addresses.
const WIDE_MAP: u16 = 16 + 64;
/// The room kept between the map and the table side by side.
const PANE_GAP: u16 = 3;

/// The station view: the address map over the station table, or beside it
/// from [`WIDE_WIDTH`] columns when the table fits next to a full-width
/// map. The table ends with `+N more` when its stations don't all fit.
fn draw_stations(frame: &mut Frame, area: Rect, sample: &MonitorSample, theme: &Theme) {
    let rows = analysis::stations(sample);
    let table = station_table(sample, &rows);
    let table_width = table.iter().map(Line::width).max().unwrap_or(0) as u16;
    if area.width >= WIDE_WIDTH && WIDE_MAP + PANE_GAP + table_width <= area.width {
        let [map_area, _, table_area] = Layout::horizontal([
            Constraint::Length(WIDE_MAP),
            Constraint::Length(PANE_GAP),
            Constraint::Min(1),
        ])
        .areas(area);
        let map = station_map(&rows, usize::from(map_area.width));
        frame.render_widget(Paragraph::new(more(map, map_area.height, theme)), map_area);
        let table = more(table, table_area.height, theme);
        frame.render_widget(Paragraph::new(table), table_area);
        return;
    }
    let mut lines = station_map(&rows, usize::from(area.width));
    lines.push(Line::raw(""));
    lines.extend(table);
    frame.render_widget(Paragraph::new(more(lines, area.height, theme)), area);
}

/// The station table's columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StationColumn {
    /// The station's MAC.
    Mac,
    /// Master, slave or router.
    Role,
    /// Its device instance, once heard.
    Device,
    /// Valid frames it sent.
    Frames,
    /// How long ago it last sent one.
    Heard,
}

/// The station table: a header and a row per station, each column as wide
/// as its widest cell.
fn station_table(sample: &MonitorSample, rows: &[analysis::StationRow]) -> Vec<Line<'static>> {
    use StationColumn as C;
    if rows.is_empty() {
        return vec![Line::raw("No station heard yet.")];
    }
    let now = sample.stats.last_activity.unwrap_or_default();
    let cells: Vec<[String; 5]> = rows
        .iter()
        .map(|row| {
            [
                row.mac.to_string(),
                row.role.label().to_string(),
                row.device.map_or_else(|| "-".into(), |d| d.to_string()),
                row.frames.to_string(),
                row.last_seen.map_or_else(
                    || "never".into(),
                    |seen| format!("{} ago", millis(now.saturating_sub(seen))),
                ),
            ]
        })
        .collect();
    let mut columns = [
        Column::new(C::Mac, "MAC"),
        Column::new(C::Role, "Role"),
        Column::new(C::Device, "Device"),
        Column::new(C::Frames, "Frames"),
        Column::new(C::Heard, "Last heard"),
    ];
    for row in &cells {
        for (column, cell) in columns.iter_mut().zip(row) {
            column.fit(text_width(cell));
        }
    }
    let line = |cells: &[String]| {
        let text = columns
            .iter()
            .zip(cells)
            .map(|(column, cell)| {
                let pad = column.width.saturating_sub(text_width(cell));
                match column.kind {
                    C::Role | C::Heard => format!("{cell}{:pad$}", ""),
                    _ => format!("{:pad$}{cell}", ""),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Line::raw(text.trim_end().to_string())
    };
    let mut lines = vec![line(&columns::headers(&columns))];
    lines.extend(cells.iter().map(|row| line(row)));
    lines
}

/// The address map, 64 addresses a row where they fit and 32 otherwise,
/// and its legend.
fn station_map(rows: &[analysis::StationRow], width: usize) -> Vec<Line<'static>> {
    let role = |mac: u8| rows.iter().find(|row| row.mac == mac).map(|row| row.role);
    let per: u8 = if width >= usize::from(WIDE_MAP) {
        64
    } else {
        32
    };
    let mut lines = Vec::new();
    for (label, first, last) in [("Masters", 0u8, 127u8), ("Slaves", 128, 254)] {
        let mut start = first;
        loop {
            let end = start.saturating_add(per - 1).min(last);
            let cells: String = (start..=end)
                .map(|mac| role(mac).map_or('.', Role::letter))
                .collect();
            let label = if start == first { label } else { "" };
            lines.push(Line::raw(format!("{label:<8}{start:>3}-{end:<3} {cells}")));
            if end == last {
                break;
            }
            start = end + 1;
        }
    }
    lines.push(Line::raw(
        "M master  S slave  R router  x silent  . nothing heard",
    ));
    lines
}

fn max_master(value: Option<MaxMaster>) -> String {
    match value {
        Some(MaxMaster::Exact(v)) => v.to_string(),
        Some(MaxMaster::AtLeast(v)) => format!("≥{v}"),
        None => "?".into(),
    }
}

fn mean_count(total: u64, count: u64) -> String {
    if count == 0 {
        return "-".into();
    }
    let tenths = total * 10 / count;
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn opt(value: Option<Duration>) -> String {
    value.map_or_else(|| "-".into(), millis)
}

/// The successor chain, rotation with its sparkline, and per-master timing.
fn ring(
    monitor: &MonitorState,
    sample: &MonitorSample,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let stats: &MonitorStats = &sample.stats;
    let chain = analysis::chain(stats);
    let chain = if chain.is_empty() {
        "no token passed yet".into()
    } else {
        chain
            .iter()
            .map(|mac| format!("mstp:{mac}"))
            .collect::<Vec<_>>()
            .join(" -> ")
    };
    let r = analysis::rotation(stats);
    let over = r.max.is_some_and(|max| max > monitor.thresholds.rotation);
    let rotation = format!(
        "Rotation    min {}  avg {}  max {}  ({} samples){}",
        opt(r.min),
        opt(r.mean),
        opt(r.max),
        r.count,
        if over { "  OVER THRESHOLD" } else { "" }
    );
    let history: VecDeque<f64> = sample
        .rotations
        .iter()
        .map(|d| d.as_secs_f64() * 1000.0)
        .collect();
    let spark = sparkline(&history, width.saturating_sub(12).max(1));
    let mut lines = vec![
        Line::raw(format!("Successors  {chain}")),
        if over {
            Line::styled(rotation, theme.warn())
        } else {
            Line::raw(rotation)
        },
        Line::raw(format!("Recent      {spark}")),
        Line::styled(
            format!(
                "Thresholds  rotation {}, hold {} (:mstp rotation|hold MS)",
                millis(monitor.thresholds.rotation),
                millis(monitor.thresholds.hold)
            ),
            theme.dim(),
        ),
        Line::raw(""),
        Line::raw(format!(
            "{:>4} {:>5} {:>7} {:>15} {:>13} {:>7} {:>6}",
            "MAC", "Next", "Tokens", "Hold avg/max", "Frames/token", "Retries", "MaxMst"
        )),
    ];
    for (mac, station) in analysis::masters(stats) {
        let hold = &station.token_hold;
        let frames = &station.frames_per_token;
        let over = hold.max.is_some_and(|max| max > monitor.thresholds.hold);
        let text = format!(
            "{mac:>4} {:>5} {:>7} {:>15} {:>13} {:>7} {:>6}{}",
            station
                .next_station
                .map_or_else(|| "?".into(), |n| n.to_string()),
            station.tokens_received,
            format!("{}/{}", opt(hold.mean()), opt(hold.max)),
            format!(
                "{}/{}",
                mean_count(frames.total, frames.count),
                frames.max.map_or_else(|| "-".into(), |m| m.to_string())
            ),
            station.token_retries,
            max_master(station.max_master),
            if over { "  LONG HOLD" } else { "" }
        );
        lines.push(if over {
            Line::styled(text, theme.warn())
        } else {
            Line::raw(text)
        });
    }
    lines.push(Line::styled(
        "MaxMst: N where its polls wrap, ≥N the highest it polled, ? none seen",
        theme.dim(),
    ));
    lines
}

/// The wizard, then the findings.
fn findings(monitor: &MonitorState, sample: &MonitorSample, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match &monitor.wizard {
        None => lines.push(Line::styled(
            "J: safe join. The monitor listens, then proposes a free MAC.",
            theme.dim(),
        )),
        Some(Wizard::Listening { since }) => {
            let heard = monitor.latest.as_deref().map_or(Duration::ZERO, |latest| {
                latest.elapsed.saturating_sub(*since)
            });
            let left = monitor.listen.saturating_sub(heard).as_secs();
            lines.push(Line::styled(
                format!("Safe join: listening, {left} s to go (Esc cancels)."),
                theme.accent(),
            ));
        }
        Some(Wizard::Failed(why)) => {
            lines.push(Line::styled(format!("Safe join: {why}"), theme.warn()));
            lines.push(Line::styled("J listens again.", theme.dim()));
        }
        Some(Wizard::Proposed { proposal, selected }) => {
            let mac = proposal.candidates[*selected];
            lines.push(Line::styled(
                format!(
                    "Safe join: MAC {mac}, Max_Master {}, Max_Info_Frames {}.",
                    proposal.max_master_for(mac),
                    monitor.max_info_frames
                ),
                theme.accent(),
            ));
            let free: Vec<String> = proposal
                .candidates
                .iter()
                .map(|c| {
                    if *c == mac {
                        format!("[{c}]")
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            lines.push(Line::raw(format!("Free and polled: {}", free.join(" "))));
            for note in &proposal.notes {
                lines.push(Line::styled(format!("Note: {note}"), theme.dim()));
            }
            lines.push(Line::styled(
                if monitor.join_allowed {
                    "+/- another MAC | Enter review and join (y) | Esc cancel"
                } else {
                    "Passive (--passive): a proposal only; this session never joins."
                },
                theme.dim(),
            ));
        }
    }
    lines.push(Line::raw(""));
    let found = analysis::findings(sample, &monitor.thresholds);
    if found.is_empty() {
        lines.push(Line::raw(
            "No findings: no Max_Master leaves a station out, and no threshold was crossed.",
        ));
    }
    for finding in found {
        let (label, style) = match finding.severity {
            Severity::Warning => ("WARN", theme.warn()),
            Severity::Notice => ("NOTE", theme.dim()),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label}  "), style),
            Span::raw(finding.text),
        ]));
    }
    lines
}

/// Error counters, lost tokens and timing quality.
fn errors(monitor: &MonitorState, sample: &MonitorSample, theme: &Theme) -> Vec<Line<'static>> {
    let since = if monitor.baseline.is_some() {
        "since clear"
    } else {
        "since start"
    };
    let state = if monitor.paused { "PAUSED" } else { "live" };
    let mut lines = vec![Line::styled(
        format!("Counters ({since}, {state}): z clear, Space pause/follow"),
        theme.dim(),
    )];
    for (label, value) in monitor.errors().unwrap_or_default() {
        lines.push(Line::raw(format!("{label:<24}{value:>10}")));
    }
    lines.push(Line::raw(format!(
        "{:<24}{:>10}  (Clause 9.10, decoded only)",
        "Extended (COBS) frames", sample.stats.cobs_frames
    )));
    lines.push(Line::raw(""));
    if sample.lost.is_empty() {
        lines.push(Line::raw("No token lost."));
    }
    for lost in &sample.lost {
        let by = lost
            .regenerated_by
            .map_or_else(|| "an unreadable frame".into(), |mac| format!("mstp:{mac}"));
        lines.push(Line::styled(
            format!(
                "Token lost at {:.1} s: silent {}, then {by}",
                lost.at.as_secs_f64(),
                millis(lost.silence)
            ),
            theme.warn(),
        ));
    }
    let timing = &sample.stats.timing;
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "Timestamps ±{} (a USB adapter's latency; lower for a native UART); up to {} \
         octets per read.",
        millis(timing.resolution),
        timing.max_octets_per_read
    )));
    lines.push(Line::styled(
        "Hardware-tested on Linux; macOS and Windows are experimental, and Windows \
         timers add to the uncertainty.",
        theme.dim(),
    ));
    lines
}
