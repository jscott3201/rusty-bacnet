//! The Network screen's Networks tab (#960): each network heard, the
//! routers that reach it, and a flag on a number more than one router
//! claims. Every state is marked by text as well as colour.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Row, Table, Wrap};

use super::Theme;
use crate::tui::app::App;
use crate::tui::app::devices::age_label;

/// Widths of the Network, Heard and flag columns.
const NETWORK: u16 = 7;
const HEARD: u16 = 9;
const FLAG: u16 = 9;
/// The narrowest the Router column gets when long router lists don't fit.
const ROUTER_MIN: u16 = 21;

/// The Networks tab's subtitle: routers heard, the local network's number,
/// and when the routers were last asked.
pub(super) fn subtitle(app: &App) -> String {
    let state = &app.network;
    let routers = match state.routers.router_count() {
        1 => "1 router".to_string(),
        n => format!("{n} routers"),
    };
    let local = match state.local_number {
        Some((network, true)) => format!(" | local network {network} (configured)"),
        Some((network, false)) => format!(" | local network {network} (learned)"),
        None => String::new(),
    };
    let asked = if app.passive_listening() {
        "listening for announcements".into()
    } else if app.passive_listener.is_some() {
        "stopped; no live input".into()
    } else {
        match state.routers_asked {
            Some(at) => format!("asked {} ago", age_label(app.now, at)),
            None => "r asks the routers".to_string(),
        }
    };
    format!("| {routers}{local} | {asked} ")
}

/// The Networks tab: each network heard, the routers that reach it, and a
/// flag on a network more than one router claims.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let map = &app.network.routers;
    if map.is_empty() {
        let text = if app.passive_listening() {
            "Listening: no router announcement overheard yet."
        } else if app.passive_listener.is_some() {
            "Listener stopped; no live input. No router announcement retained."
        } else {
            "No router heard yet. r asks every router for the networks it reaches."
        };
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), area);
        return;
    }
    let rows = map.rows();
    // The Router column is as wide as its longest entry, so Heard sits
    // beside it rather than at the far edge.
    let routers: Vec<String> = rows.iter().map(|row| row.routers.join(", ")).collect();
    let longest = routers
        .iter()
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(0)
        .max("Router".len());
    let fixed = NETWORK + HEARD + FLAG + 3;
    let router_width = u16::try_from(longest)
        .unwrap_or(u16::MAX)
        .min(area.width.saturating_sub(fixed).max(ROUTER_MIN));
    let rows: Vec<Row> = rows
        .into_iter()
        .zip(routers)
        .map(|(row, routers)| {
            let flag = if row.duplicate() { "DUPLICATE" } else { "" };
            let line = Row::new([
                row.network.to_string(),
                routers,
                age_label(app.now, row.heard),
                flag.to_string(),
            ]);
            if row.duplicate() {
                line.style(theme.danger())
            } else {
                line
            }
        })
        .collect();
    let widths = [
        Constraint::Length(NETWORK),
        Constraint::Length(router_width),
        Constraint::Length(HEARD),
        Constraint::Length(FLAG),
    ];
    let text = if app.passive_listener.is_some() {
        "DUPLICATE: several overheard routers claim one network number; a shared number, loop or moved route."
    } else {
        "DUPLICATE: several routers claim one network number: a shared \
                number, a loop, or a moved route (r asks again)."
    };
    let note_height = if map.duplicates().is_empty() {
        0
    } else if text.len() <= usize::from(area.width) {
        1
    } else {
        // Wrapped: two lines hold it from 80 columns.
        2
    };
    let [table, note] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(note_height)]).areas(area);
    let header = Row::new(["Network", "Router", "Heard", ""]).style(theme.accent());
    frame.render_widget(
        Table::new(rows, widths).header(header).column_spacing(1),
        table,
    );
    if note_height > 0 {
        let note_text = Paragraph::new(Line::styled(text, theme.warn())).wrap(Wrap { trim: true });
        frame.render_widget(note_text, note);
    }
}
