//! The Capture screen: the frame list on top; below it the selected frame's
//! decode tree and its octets in hex, the selected field's marked. From 120
//! columns the tree and the hex view sit side by side; below that the
//! detail pane shows the tree, or the hex view when it has the focus.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table};
use tui_tree_widget::Tree;

use super::{FULL_WIDTH, Theme};
use crate::tui::app::capture::{CaptureState, Focus};
use crate::tui::app::{App, Link};
use crate::tui::capture::CaptureFrame;

/// `HH:MM:SS.mmm`, UTC.
fn clock(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let day = since.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        day / 3_600,
        day % 3_600 / 60,
        day % 60,
        since.subsec_millis()
    )
}

/// A short name for the link message.
fn link_short(link: &str) -> &str {
    match link {
        "original-unicast-npdu" => "unicast",
        "original-broadcast-npdu" => "bcast",
        "forwarded-npdu" => "fwd",
        "distribute-broadcast-to-network" => "dist-bcast",
        "bvlc-result" => "result",
        "register-foreign-device" => "reg-fd",
        "encapsulated-npdu" => "encap",
        "bacnet-data-expecting-reply" | "bacnet-extended-data-expecting-reply" => "data-er",
        "bacnet-data-not-expecting-reply" | "bacnet-extended-data-not-expecting-reply" => {
            "data-ner"
        }
        "poll-for-master" => "pfm",
        "reply-to-poll-for-master" => "reply-pfm",
        "reply-postponed" => "postponed",
        other => other,
    }
}

/// A short name for the APDU type.
fn pdu_short(pdu: &str) -> &str {
    match pdu {
        "Confirmed-Request" => "conf-req",
        "Unconfirmed-Request" => "unconf-req",
        "Simple-ACK" => "simple-ack",
        "Complex-ACK" => "cmplx-ack",
        "Segment-ACK" => "seg-ack",
        "Error" => "error",
        "Reject" => "reject",
        "Abort" => "abort",
        other => other,
    }
}

/// The network and application layers in a few words.
fn pdu_text(frame: &CaptureFrame) -> String {
    let summary = &frame.summary;
    match (&summary.pdu, &summary.network_message) {
        (Some(pdu), _) => pdu_short(pdu).to_string(),
        (None, Some(message)) => format!("nl {message}"),
        (None, None) => String::new(),
    }
}

/// How much of the list fits: below 120 columns the link, direction,
/// message, APDU type, invoke ID and service fold into a direction mark and
/// one `Info` column; from 150 a `Via` column shows a routed frame's link
/// peer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Width {
    Narrow,
    Wide,
    Widest,
}

/// From this many columns the list shows `Via`.
const VIA_WIDTH: u16 = 150;

/// Columns: (header, narrow, wide, widest width); 0 hides it. The last
/// column takes what is left.
const COLUMNS: [(&str, u16, u16, u16); 12] = [
    ("No.", 5, 6, 6),
    ("Time", 12, 12, 12),
    ("Link", 0, 4, 4),
    ("Dir", 0, 3, 3),
    ("", 1, 0, 0),
    ("Source", 15, 19, 21),
    ("Destination", 15, 19, 21),
    ("Via", 0, 0, 21),
    ("Message", 0, 9, 9),
    ("PDU", 0, 10, 10),
    ("Inv", 0, 3, 3),
    ("Service", 0, 0, 0),
];

fn width_of(column: &(&str, u16, u16, u16), width: Width) -> u16 {
    match width {
        Width::Narrow => column.1,
        Width::Wide => column.2,
        Width::Widest => column.3,
    }
}

fn cells(number: u64, frame: &CaptureFrame, width: Width) -> Vec<String> {
    let summary = &frame.summary;
    let invoke = summary
        .invoke_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let service = summary.service.clone().unwrap_or_default();
    let device = summary.device.map(|d| format!("device {d}"));
    let problem = summary.problem.clone().or(device).unwrap_or_default();
    let mark = match frame.direction {
        crate::tui::capture::Direction::In => "←",
        crate::tui::capture::Direction::Out => "→",
        crate::tui::capture::Direction::Wire => "-",
    };
    let fixed = [
        number.to_string(),
        clock(frame.time),
        frame.framing.label().to_string(),
        frame.direction.label().to_string(),
        mark.to_string(),
        frame.source.clone(),
        frame.destination.clone(),
        frame.via.clone().unwrap_or_default(),
        link_short(&summary.link).to_string(),
        pdu_text(frame),
        invoke.clone(),
    ];
    let mut out: Vec<String> = fixed
        .into_iter()
        .zip(COLUMNS)
        .filter(|(_, column)| width_of(column, width) > 0)
        .map(|(text, _)| text)
        .collect();
    let last = if width == Width::Narrow {
        // The invoke ID first, so it survives a cut; the link message only
        // when nothing above it says more.
        let pdu = pdu_text(frame);
        let mut parts = Vec::new();
        if !invoke.is_empty() {
            parts.push(format!("#{invoke}"));
        }
        parts.push(match pdu.is_empty() {
            true => link_short(&summary.link).to_string(),
            false => pdu,
        });
        parts.extend([service, problem]);
        parts.retain(|part| !part.is_empty());
        parts.join(" ")
    } else {
        [service, problem].join(" ").trim().to_string()
    };
    out.push(last);
    out
}

fn title(capture: &CaptureState) -> String {
    let shown = capture.shown();
    let held = capture.ring.len();
    let mut title = if shown == held {
        format!(" Capture {held}")
    } else {
        format!(" Capture {shown} of {held}")
    };
    title.push_str(&format!(" | dropped {}", capture.lost));
    match capture.paused {
        Some(_) => title.push_str(&format!(" | PAUSED +{} ", capture.held_back())),
        None => title.push_str(" | LIVE "),
    }
    title
}

/// What feeds the list, or why nothing does.
fn source_line(app: &App) -> String {
    let capture = &app.capture;
    if !capture.sources.is_empty() {
        let names: Vec<&str> = capture.sources.iter().map(|(s, _)| s.as_str()).collect();
        return format!(" {} ", names.join("; "));
    }
    if let Some(why) = &capture.ended {
        return format!(" stopped: {why} ");
    }
    match app.link {
        Link::Up { .. } => " starting the tap ".into(),
        Link::Passive if app.monitor.is_some() => " starting the monitor's stream ".into(),
        _ => " no source: connect, or start the MS/TP monitor ".into(),
    }
}

/// The Capture screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let capture = &app.capture;
    let wide = area.width >= FULL_WIDTH;
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Percentage(45), Constraint::Min(6)]).areas(area);
    list(frame, list_area, app, wide, theme);
    let border = |focus: Focus| {
        if capture.focus == focus {
            theme.accent()
        } else {
            theme.dim()
        }
    };
    if wide {
        let [tree_area, hex_area] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(detail_area);
        tree(frame, tree_area, capture, border(Focus::Tree), theme);
        hex(frame, hex_area, capture, border(Focus::Hex), theme);
    } else if capture.focus == Focus::Hex {
        hex(frame, detail_area, capture, border(Focus::Hex), theme);
    } else {
        tree(frame, detail_area, capture, border(Focus::Tree), theme);
    }
}

fn list(frame: &mut Frame, area: Rect, app: &App, wide: bool, theme: &Theme) {
    let capture = &app.capture;
    let mut block = Block::bordered()
        .title(Line::styled(title(capture), theme.accent()))
        .title_bottom(Line::styled(source_line(app), theme.dim()))
        .title_bottom(
            Line::styled(format!(" ring {} ", capture.ring.capacity()), theme.dim())
                .alignment(Alignment::Right),
        )
        .border_style(if capture.focus == Focus::List {
            theme.accent()
        } else {
            theme.dim()
        });
    if !capture.filter.is_empty() {
        block = block.title(
            Line::styled(format!(" filter: {} ", capture.filter), theme.warn())
                .alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let shown = capture.shown();
    if shown == 0 {
        let text = if capture.ring.len() == 0 {
            "No frames yet."
        } else {
            "No frame matches the filter."
        };
        let lines = vec![
            Line::styled(text, theme.dim()),
            Line::styled(
                "/ filters: dev:N mac:TEXT net:N svc:NAME pdu:TYPE errors bcast TEXT",
                theme.dim(),
            ),
        ];
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
        return;
    }
    let height = usize::from(inner.height.saturating_sub(1)).max(1);
    capture.page.set(height);
    let selected = capture.selected();
    let index = selected
        .and_then(|n| capture.index_of(n))
        .unwrap_or(shown - 1);
    // Keep the selection in view, moving the window as little as possible.
    let mut offset = capture.offset.get().min(shown - 1);
    if index < offset {
        offset = index;
    } else if index >= offset + height {
        offset = index + 1 - height;
    }
    capture.offset.set(offset);
    let width = match (wide, area.width >= VIA_WIDTH) {
        (false, _) => Width::Narrow,
        (true, false) => Width::Wide,
        (true, true) => Width::Widest,
    };
    let fixed: Vec<(&str, u16)> = COLUMNS
        .iter()
        .map(|column| (column.0, width_of(column, width)))
        .filter(|(h, w)| *w > 0 || *h == "Service")
        .collect();
    let mut widths = vec![Constraint::Length(1)];
    widths.extend(fixed.iter().map(|(h, w)| match *h {
        "Service" => Constraint::Min(8),
        _ => Constraint::Length(*w),
    }));
    let mut header = vec![String::new()];
    header.extend(fixed.iter().map(|(h, _)| match *h {
        "Service" if !wide => "Info".to_string(),
        "Service" => "Service / problem".to_string(),
        h => h.to_string(),
    }));
    let rows: Vec<Row> = (offset..(offset + height).min(shown))
        .filter_map(|i| capture.row(i))
        .filter_map(|n| capture.ring.get(n).map(|f| (n, f)))
        .map(|(n, f)| {
            let mark = if Some(n) == selected { ">" } else { " " };
            let mut texts = vec![mark.to_string()];
            texts.extend(cells(n, f, width));
            let row = Row::new(texts);
            match (Some(n) == selected, f.summary.problem.is_some()) {
                (true, _) => row.style(theme.selected()),
                (false, true) => row.style(theme.warn()),
                (false, false) => row,
            }
        })
        .collect();
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, inner);
}

fn tree(
    frame: &mut Frame,
    area: Rect,
    capture: &CaptureState,
    border: ratatui::style::Style,
    theme: &Theme,
) {
    let title = match &capture.detail {
        Some(detail) => format!(" Frame {}{} ", detail.number, redaction(capture)),
        None => " Frame ".to_string(),
    };
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(border);
    let Some(detail) = &capture.detail else {
        let text = Line::styled("Select a frame to decode it.", theme.dim());
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    };
    match Tree::new(&detail.items) {
        Ok(widget) => {
            // Selecting again keeps the selection in view after a resize.
            let mut state = capture.tree.borrow_mut();
            let selected = state.selected().to_vec();
            state.select(selected);
            let widget = widget
                .block(block)
                .highlight_style(theme.selected())
                .highlight_symbol("> ")
                .node_open_symbol("- ")
                .node_closed_symbol("+ ")
                .node_no_children_symbol("  ");
            frame.render_stateful_widget(widget, area, &mut *state);
        }
        Err(error) => {
            let text = Line::styled(format!("Can't draw the tree: {error}"), theme.warn());
            frame.render_widget(Paragraph::new(text).block(block), area);
        }
    }
}

/// `(password redacted)` after a title when the frame shown had its
/// password masked before it was kept (#1799).
fn redaction(capture: &CaptureState) -> &'static str {
    let redacted = capture
        .detail
        .as_ref()
        .and_then(|d| capture.ring.get(d.number))
        .is_some_and(|f| f.redacted);
    if redacted { " (password redacted)" } else { "" }
}

fn hex(
    frame: &mut Frame,
    area: Rect,
    capture: &CaptureState,
    border: ratatui::style::Style,
    theme: &Theme,
) {
    let field = capture.selected_field();
    let note = redaction(capture);
    let title = match field {
        Some(f) if f.range.len() == 1 => format!(" Hex: octet {}{note} ", f.range.start),
        Some(f) if !f.range.is_empty() => {
            format!(" Hex: octets {}-{}{note} ", f.range.start, f.range.end - 1)
        }
        _ => format!(" Hex{note} "),
    };
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(octets) = capture
        .detail
        .as_ref()
        .and_then(|d| capture.ring.get(d.number))
        .map(|f| &f.bytes)
    else {
        return;
    };
    if octets.is_empty() {
        let text = Line::styled("No octets: the monitor reported a line error.", theme.dim());
        frame.render_widget(Paragraph::new(text), inner);
        return;
    }
    // Offset, 16 octets in hex and as text, or 8 where that doesn't fit.
    let per_row = if inner.width >= 70 { 16 } else { 8 };
    let marked = field.map(|f| f.range.clone()).unwrap_or(0..0);
    let rows = octets.len().div_ceil(per_row);
    let height = usize::from(inner.height).max(1);
    // Start at the marked field's first row, so it is in view.
    let first = (marked.start / per_row).min(rows.saturating_sub(height));
    let style = |at: usize| {
        if marked.contains(&at) {
            theme.selected()
        } else {
            ratatui::style::Style::new()
        }
    };
    let lines: Vec<Line> = (first..rows.min(first + height))
        .map(|row| {
            let start = row * per_row;
            let chunk = &octets[start..(start + per_row).min(octets.len())];
            let mut spans = vec![Span::styled(format!("{start:04X} "), theme.dim())];
            for (i, b) in chunk.iter().enumerate() {
                spans.push(Span::styled(format!("{b:02X}"), style(start + i)));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::raw("   ".repeat(per_row - chunk.len())));
            spans.push(Span::raw(" "));
            for (i, b) in chunk.iter().enumerate() {
                let ch = if b.is_ascii_graphic() {
                    *b as char
                } else {
                    '.'
                };
                spans.push(Span::styled(ch.to_string(), style(start + i)));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
