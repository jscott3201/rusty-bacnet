//! The `:find` panel (#960), below the device table or, from
//! [`WIDE_WIDTH`] columns when both fit at their content widths, beside it
//! (#1915). Every state is marked by text as well as colour.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};
use ratatui::Frame;

use super::columns::{self, Column};
use super::{ellipsize, sanitize, text_width, Theme, FULL_WIDTH, WIDE_WIDTH};
use crate::tui::app::devices::age_label;
use crate::tui::app::objects::object_label;
use crate::tui::app::who_has::WhoHasSearch;
use crate::tui::app::App;
use crate::tui::network::discovery::FoundRow;

/// The most result rows the Who-Has panel takes room for.
const PANEL_ROWS: usize = 6;

/// The title's age as wide as it gets in practice: [`age_label`] counts
/// hours past an hour, so four columns cover a search up to 41 days old.
const WIDEST_AGE: &str = "100h";

/// The Devices screen: the table, and the `:find` results below it or
/// beside it.
pub(super) fn devices(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let full = area.width >= FULL_WIDTH;
    let Some(search) = &app.who_has else {
        super::devices::table(frame, area, app, theme, full);
        return;
    };
    if let Some(width) = beside(area.width, app, search) {
        let [table, panel_area] =
            Layout::horizontal([Constraint::Length(width), Constraint::Min(0)]).areas(area);
        super::devices::table(frame, table, app, theme, full);
        panel(frame, panel_area, app, search, theme, full);
        return;
    }
    // Borders and the header row, around at least one line.
    let rows = search.found.len().clamp(1, PANEL_ROWS) as u16 + 3;
    let [table, panel_area] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(rows)]).areas(area);
    super::devices::table(frame, table, app, theme, full);
    panel(frame, panel_area, app, search, theme, full);
}

/// The device table's width when the panel goes beside it: from
/// [`WIDE_WIDTH`] columns, when the table at its content width leaves the
/// panel room for its own. The table's Name column counts at its least
/// width there, so names being read don't move the panel; the table then
/// takes its whole content width where the panel leaves room for it, and
/// the panel the rest.
fn beside(width: u16, app: &App, search: &WhoHasSearch) -> Option<u16> {
    if width < WIDE_WIDTH {
        return None;
    }
    let (least, content) = super::devices::widths(width, app);
    let room = usize::from(width).checked_sub(content_width(search))?;
    (least <= room).then_some(content.min(room) as u16)
}

/// The panel's title: what was searched for, how many answered, when.
fn title(search: &WhoHasSearch, age: &str) -> String {
    let found = match search.found.len() {
        1 => "1 found".to_string(),
        n => format!("{n} found"),
    };
    format!(
        " Who-Has \"{}\" | {found} | sent {age} ago | :find closes ",
        search.name
    )
}

/// What the panel shows with no result.
fn empty_text(search: &WhoHasSearch) -> String {
    match &search.failed {
        Some(error) => format!("Not sent: {error}"),
        None => "No I-Have yet.".to_string(),
    }
}

/// The columns the panel takes at its content width, borders included:
/// every name in full and its title, but not the Via column, which goes
/// in only when the room is there. The age in the title is counted at
/// [`WIDEST_AGE`], so the panes don't move as it counts.
fn content_width(search: &WhoHasSearch) -> usize {
    let body = if search.found.is_empty() {
        text_width(&empty_text(search))
    } else {
        let mut columns = columns(u16::MAX, true, &search.found);
        columns.retain(|column| column.kind != Field::Via);
        columns::total(&columns)
    };
    body.max(text_width(&title(search, WIDEST_AGE))) + 2
}

/// The bordered panel of results in `area`.
fn panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    search: &WhoHasSearch,
    theme: &Theme,
    full: bool,
) {
    let title = title(search, &age_label(app.now, search.sent));
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if search.found.is_empty() {
        let style = match search.failed {
            Some(_) => theme.danger(),
            None => theme.dim(),
        };
        let text = Line::styled(empty_text(search), style);
        frame.render_widget(Paragraph::new(text), inner);
        return;
    }
    let columns = columns(area.width, full, &search.found);
    let rows: Vec<Row> = search
        .found
        .iter()
        .map(|row| {
            let cells = columns.iter().map(|column| match column.kind {
                Field::Name => ellipsize(&cell(Field::Name, row), column.width),
                kind => cell(kind, row),
            });
            Row::new(cells.collect::<Vec<_>>())
        })
        .collect();
    let table = Table::new(rows, columns::constraints(&columns))
        .header(Row::new(columns::headers(&columns)).style(theme.accent()))
        .column_spacing(columns::SPACING);
    frame.render_widget(table, inner);
}

/// What a column of the results shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The device's instance.
    Device,
    /// The object, as the browser labels it.
    Object,
    /// The object's name, from the I-Have.
    Name,
    /// The device's address, or a routed device's station.
    From,
    /// The router a routed I-Have came through.
    Via,
    /// Its network, or `local`.
    Network,
}

/// The narrowest the Name column gets: its header.
const MIN_NAME: usize = "Name".len();

/// One cell's text, before it is fitted to its column. The name is the
/// device's own text, so control characters in it become spaces.
fn cell(field: Field, row: &FoundRow) -> String {
    match field {
        Field::Device => row.device.to_string(),
        Field::Object => object_label(row.object),
        Field::Name => sanitize(&row.name),
        Field::From => row.address.clone(),
        Field::Via => row.via.clone().unwrap_or_default(),
        Field::Network => row
            .network
            .map_or_else(|| "local".to_string(), |n| n.to_string()),
    }
}

/// The columns for a panel `width` wide, sized to the results; `full` (a
/// terminal [`FULL_WIDTH`] columns wide) spells out Network. Device,
/// object, address and network are never cut; the name takes the room
/// left, cut with `…` when it is longer, and the Via column appears only
/// when the whole of it fits beside every name in full. Unlike the device
/// table, where Via goes in after the vendor has given way, here Via is
/// placed before the name shrinks: the name has the room first, and the
/// router is left out rather than a name cut to make space for it.
fn columns(width: u16, full: bool, found: &[FoundRow]) -> Vec<Column<Field>> {
    let network = if full { "Network" } else { "Net" };
    let mut columns = vec![
        Column::new(Field::Device, "Device"),
        Column::new(Field::Object, "Object"),
        Column::new(Field::Name, "Name"),
        Column::new(Field::From, "From"),
        Column::new(Field::Network, network),
    ];
    let mut via = Column::new(Field::Via, "Via");
    for row in found {
        for column in &mut columns {
            column.fit(text_width(&cell(column.kind, row)));
        }
        if let Some(router) = &row.via {
            via.fit(text_width(router));
        }
    }
    // Inside the borders.
    let room = usize::from(width.saturating_sub(2));
    if found.iter().any(|row| row.via.is_some()) {
        columns::insert_if_room(&mut columns, 4, via, room);
    }
    columns::shrink(&mut columns, Field::Name, room, MIN_NAME);
    columns
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;
    use crate::tui::message::WorkerEvent;
    use crate::tui::tests::{connected_app, row, worker};

    /// The panel goes beside the table when their widths add up to exactly
    /// the terminal's, and below it one column short of that.
    #[test]
    fn beside_needs_exactly_both_widths() {
        let mut app = connected_app();
        for (instance, address) in [(1001, "10.0.1.21:47808"), (1002, "10.0.1.22:47808")] {
            worker(
                &mut app,
                WorkerEvent::Discovered(row(instance, address, 260, 2)),
            );
        }
        app.settle();
        let name = "Building 4 East Wing AHU-07 Supply Air Temp Setpoint";
        let search = WhoHasSearch {
            name: name.into(),
            sent: app.now,
            found: vec![FoundRow {
                device: 1001,
                object: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3).unwrap(),
                name: name.into(),
                address: "10.0.1.21:47808".into(),
                via: None,
                network: None,
            }],
            failed: None,
        };
        let (least, _) = crate::tui::view::devices::widths(200, &app);
        let need = (least + content_width(&search)) as u16;
        assert!(need >= WIDE_WIDTH, "{need}");
        assert!(beside(need, &app, &search).is_some());
        assert_eq!(beside(need - 1, &app, &search), None);
    }
}
