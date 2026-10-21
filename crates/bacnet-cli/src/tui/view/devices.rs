//! The Devices screen: the duplicate-instance banner and the live table.

use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use super::columns::{self, Column};
use super::{ellipsize, name_marker, text_width, Names, Theme, FULL_WIDTH, MIN_NAME};
use crate::tui::app::devices::{age_label, segmentation_label, SortKey};
use crate::tui::app::names::device_object;
use crate::tui::app::{App, Link, OpState};
use crate::tui::message::DeviceRow;

/// The banners (duplicate instances, the foreign-device registration), as
/// lines [`wrap_banner`] has already wrapped; the model keeps their text up
/// to date.
pub(super) fn banner(frame: &mut Frame, area: Rect, lines: &[String], theme: &Theme) {
    if lines.is_empty() {
        return;
    }
    let text: Vec<Line> = lines
        .iter()
        .map(|l| Line::styled(l.as_str(), theme.danger()))
        .collect();
    frame.render_widget(Paragraph::new(text).style(theme.danger()), area);
}

/// One banner as lines at most `width` columns wide, broken between words
/// (a word longer than a line is split) and each continuation indented one
/// space, so the banner's height is exactly the number of lines.
pub(super) fn wrap_banner(text: &str, width: usize) -> Vec<String> {
    const INDENT: &str = " ";
    let width = width.max(INDENT.len() + 1);
    let lead = &text[..text.len() - text.trim_start().len()];
    let mut lines = Vec::new();
    let mut line = lead.to_string();
    let mut fresh = true;
    for word in text.split_whitespace() {
        let len = |s: &str| s.chars().count();
        let sep = usize::from(!fresh);
        if len(&line) + sep + len(word) > width && !fresh {
            lines.push(std::mem::replace(&mut line, INDENT.to_string()));
            fresh = true;
        }
        if !fresh {
            line.push(' ');
        }
        let mut rest: Vec<char> = word.chars().collect();
        // A word too long for any line fills lines whole.
        while len(&line) + rest.len() > width {
            let take = width - len(&line);
            line.extend(rest.drain(..take));
            lines.push(std::mem::replace(&mut line, INDENT.to_string()));
        }
        line.extend(rest);
        fresh = false;
    }
    lines.push(line);
    lines
}

/// Text for the running or last Who-Is, for the table's bottom border.
fn op_status(app: &App) -> Option<String> {
    let op = app.op.as_ref()?;
    let what = format!("Who-Is {}", op.spec.describe());
    let replies = match op.replies {
        1 => "1 reply".to_string(),
        n => format!("{n} replies"),
    };
    Some(match &op.state {
        OpState::Sending => format!("{what}: sending"),
        OpState::Listening { until } => {
            let left = until.saturating_duration_since(app.now);
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            format!("{what}: listening {secs}s, {replies}")
        }
        OpState::Done => format!("{what}: done, {replies}"),
        OpState::Cancelled => format!("{what}: cancelled, {replies}"),
        OpState::Failed(error) => format!("{what} failed: {error}"),
        OpState::CancelFailed(error) => format!("{what} cancel failed: {error}"),
    })
}

fn title(app: &App) -> String {
    let (key, descending) = app.devices.sort();
    let arrow = if descending { "↓" } else { "↑" };
    let shown = app.devices.visible_len();
    let total = app.devices.len();
    let count = if shown == total {
        total.to_string()
    } else {
        format!("{shown} of {total}")
    };
    format!(" Devices ({count}) | sort: {} {arrow} ", key.label())
}

/// What a column of the table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The selection marker.
    Marker,
    /// Device instance.
    Instance,
    /// The device's Object_Name (#1896).
    Name,
    /// Its address, or a routed device's station, as a target.
    Address,
    /// The router a routed device answered through.
    Via,
    /// Its network, or `local`.
    Network,
    /// Vendor.
    Vendor,
    /// Max APDU length accepted.
    Apdu,
    /// Segmentation supported.
    Segmentation,
    /// Age of the last I-Am.
    Seen,
}

/// The columns for a table `width` columns wide, each sized to its content
/// over every visible row (not just those on screen, so scrolling doesn't
/// move them). With `full` (a terminal [`FULL_WIDTH`] columns wide or more,
/// even when the Who-Has panel takes some of it) the headers are spelled
/// out. Instance and address are never cut, and the Via column appears
/// only when the whole of it fits.
///
/// The vendor column is the one that gives way when a row is too wide. A
/// vendor ID (at most five digits) never needs to; the shrink is there for
/// vendor names, should the table show them. It stops at the header's own
/// width, sort marker included, so the header is never clipped.
///
/// The Name column (#1896) comes last: it takes the room the others leave,
/// a long name ending in `…`, and is left out below [`MIN_NAME`] columns
/// (at 80 columns with long addresses, say), when its names aren't asked for.
/// Sorted by name, it always shows: the columns in [`GIVES_WAY`] are left
/// out to make room for it.
fn columns(width: u16, full: bool, app: &App) -> Vec<Column<Field>> {
    let sorted = sorted_field(app.devices.sort().0);
    let column = |field: Field, short: &str, long: &str| {
        let mut header = if full { long } else { short }.to_string();
        if field == sorted {
            header.push('*');
        }
        Column::new(field, header)
    };
    let mut columns = vec![
        column(Field::Marker, "", ""),
        column(Field::Instance, "Instance", "Instance"),
        column(Field::Address, "Address", "Address"),
        column(Field::Network, "Net", "Network"),
        column(Field::Vendor, "Vendor", "Vendor"),
        column(Field::Apdu, "APDU", "Max APDU"),
        column(Field::Segmentation, "Seg", "Segmentation"),
        column(Field::Seen, "Seen", "Last seen"),
    ];
    let mut via = column(Field::Via, "Via", "Via");
    let mut name = column(Field::Name, "Name", "Name");
    let mut routed = false;
    for row in app.devices.visible() {
        name.fit(cell_width(Field::Name, row, app));
        for column in &mut columns {
            column.fit(cell_width(column.kind, row, app));
        }
        if let Some(router) = &row.via {
            routed = true;
            via.fit(text_width(router));
        }
    }
    let min_vendor = columns
        .iter()
        .find(|column| column.kind == Field::Vendor)
        .map_or(0, |column| text_width(&column.header));
    // Inside the borders.
    let room = usize::from(width.saturating_sub(2));
    columns::shrink(&mut columns, Field::Vendor, room, min_vendor);
    if app.devices.sort().0 == SortKey::Name {
        // The sorted column always shows: the others give way, rightmost
        // first, then the Via column if there is still room for it.
        let need = usize::from(columns::SPACING) + MIN_NAME.min(name.width);
        while columns::total(&columns) + need > room {
            let Some(at) = columns.iter().rposition(|c| GIVES_WAY.contains(&c.kind)) else {
                break;
            };
            columns.remove(at);
        }
        columns::insert_narrowed(&mut columns, 2, name, room, MIN_NAME);
        if routed {
            columns::insert_if_room(&mut columns, 4, via, room);
        }
        return columns;
    }
    if routed {
        columns::insert_if_room(&mut columns, 3, via, room);
    }
    columns::insert_narrowed(&mut columns, 2, name, room, MIN_NAME);
    columns
}

/// The columns left out, rightmost first, to make room for the Name
/// column while the table is sorted by name.
const GIVES_WAY: [Field; 5] = [
    Field::Network,
    Field::Vendor,
    Field::Apdu,
    Field::Segmentation,
    Field::Seen,
];

/// One cell's text, before it is fitted to its column.
fn cell(field: Field, row: &DeviceRow, app: &App, selected: bool) -> String {
    match field {
        Field::Marker => if selected { ">" } else { " " }.to_string(),
        Field::Instance => row.instance.to_string(),
        Field::Name => name(row, app).to_string(),
        Field::Address => row.address.clone(),
        Field::Via => row.via.clone().unwrap_or_default(),
        Field::Network => row
            .network
            .map_or_else(|| "local".to_string(), |n| n.to_string()),
        Field::Vendor => row.vendor_id.to_string(),
        Field::Apdu => row.max_apdu.to_string(),
        Field::Segmentation => segmentation_label(row.segmentation),
        Field::Seen => age_label(app.now, row.last_seen),
    }
}

/// The device's name as its cell shows it.
fn name<'a>(row: &DeviceRow, app: &'a App) -> &'a str {
    Names::of(app).text(row.instance, device_object(row.instance))
}

/// The width of [`cell`]'s text, without building it where the cell is a
/// number or a string the row already holds: the columns are measured over
/// every visible row on every frame.
fn cell_width(field: Field, row: &DeviceRow, app: &App) -> usize {
    let digits = |n: u32| n.checked_ilog10().map_or(1, |d| d as usize + 1);
    match field {
        Field::Marker => 1,
        Field::Instance => digits(row.instance),
        Field::Name => text_width(name(row, app)),
        Field::Address => text_width(&row.address),
        Field::Via => row.via.as_deref().map_or(0, text_width),
        Field::Network => row.network.map_or("local".len(), |n| digits(n.into())),
        // A vendor name (#1896) will need text_width here.
        Field::Vendor => digits(row.vendor_id.into()),
        Field::Apdu => digits(row.max_apdu),
        Field::Segmentation | Field::Seen => text_width(&cell(field, row, app, false)),
    }
}

/// A row's cells, each fitted to its column: a vendor too wide for its
/// column ends in `…`, and a name marker (`…`, `?`) is dim.
fn cells<'a>(
    columns: &[Column<Field>],
    row: &DeviceRow,
    app: &App,
    selected: bool,
    theme: &Theme,
) -> Vec<Cell<'a>> {
    columns
        .iter()
        .map(|column| {
            let text = cell(column.kind, row, app, selected);
            match column.kind {
                Field::Name if name_marker(&text) => Cell::new(text).style(theme.dim()),
                Field::Vendor | Field::Name => Cell::new(ellipsize(&text, column.width)),
                _ => Cell::new(text),
            }
        })
        .collect()
}

/// The filter's title on the table's top border, when one is set.
fn filter_title(app: &App) -> Option<String> {
    let filter = app.devices.filter();
    (!filter.is_empty()).then(|| format!(" filter: \"{filter}\" "))
}

/// The table's widths in a terminal `width` wide (#1915), borders
/// included: `(least, content)`. Each covers its columns (or the text it
/// shows when it has no row) and its top titles. `content` has every name
/// read so far in full; `least` counts the Name column at [`MIN_NAME`]
/// whatever names are known, so a layout chosen by it stays put as names
/// are read or dropped. The Who-Is status on the bottom border is left
/// out, so a long error doesn't move the panes either.
pub(super) fn widths(width: u16, app: &App) -> (usize, usize) {
    let titles = text_width(&title(app)) + filter_title(app).map_or(0, |f| 1 + text_width(&f));
    if app.devices.visible_len() == 0 {
        let both = text_width(&empty_text(app)).max(titles) + 2;
        return (both, both);
    }
    let columns = columns(width, width >= FULL_WIDTH, app);
    let content = columns::total(&columns);
    let least = match columns.iter().find(|column| column.kind == Field::Name) {
        Some(name) => content - name.width + MIN_NAME,
        None => content,
    };
    (least.max(titles) + 2, content.max(titles) + 2)
}

pub(super) fn table(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, full: bool) {
    let mut block = Block::bordered()
        .title(Line::styled(title(app), theme.accent()))
        .border_style(theme.dim());
    if let Some(filter) = filter_title(app) {
        block = block.title(Line::styled(filter, theme.warn()).alignment(Alignment::Right));
    }
    if let Some(status) = op_status(app) {
        block = block.title_bottom(Line::from(format!(" {status} ")));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.devices.visible_len() == 0 {
        empty(frame, inner, app, theme);
        return;
    }
    let columns = columns(area.width, full, app);
    let named = columns.iter().any(|column| column.kind == Field::Name);
    app.devices.named.set(named);
    // One line for the header row.
    let height = usize::from(inner.height.saturating_sub(1)).max(1);
    let offset = scroll(app, height);
    let selected = app.devices.selected_index();
    let rows: Vec<Row> = app
        .devices
        .visible_window(offset, height)
        .enumerate()
        .map(|(i, row)| {
            let is_selected = selected == Some(offset + i);
            let line = Row::new(cells(&columns, row, app, is_selected, theme));
            if is_selected {
                line.style(theme.selected())
            } else {
                line
            }
        })
        .collect();
    let table = Table::new(rows, columns::constraints(&columns))
        .header(Row::new(columns::headers(&columns)).style(theme.accent()))
        .column_spacing(columns::SPACING);
    frame.render_widget(table, inner);
}

/// The column a sort key orders by, whose header is marked.
fn sorted_field(key: SortKey) -> Field {
    match key {
        SortKey::Instance => Field::Instance,
        SortKey::Name => Field::Name,
        SortKey::Address => Field::Address,
        SortKey::Network => Field::Network,
        SortKey::Vendor => Field::Vendor,
        SortKey::LastSeen => Field::Seen,
    }
}

/// First visible row, keeping the selection on screen. The table remembers
/// it between frames so the view only scrolls when the selection leaves it.
fn scroll(app: &App, height: usize) -> usize {
    let table = &app.devices;
    table.page.set(height);
    let mut offset = table.offset.get();
    if let Some(selected) = table.selected_index() {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + height {
            offset = selected + 1 - height;
        }
    }
    offset = offset.min(table.visible_len().saturating_sub(height));
    table.offset.set(offset);
    offset
}

fn empty(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = empty_text(app);
    let lines = vec![Line::raw(""), Line::from(Span::styled(text, theme.dim()))];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// What the table says when it has no row to show.
fn empty_text(app: &App) -> String {
    match (&app.link, app.devices.len()) {
        (Link::Down, _) => "Disconnected. Press p for profiles or c to connect.".into(),
        (Link::Disconnecting, _) => "Disconnecting; waiting for local resources to stop...".into(),
        (Link::Picking, _) => String::new(),
        (Link::Connecting, _) => "Connecting...".to_string(),
        (Link::Failed(error), _) => format!("Connection failed: {}", error.detail),
        (Link::Passive, _) => {
            "Passive: no network client; nothing is transmitted. u shows the ledger.".into()
        }
        (Link::Up { .. }, 0) => "No devices yet. Press d to send a Who-Is.".to_string(),
        (Link::Up { .. }, _) => format!(
            "No device matches \"{}\". Esc clears the filter.",
            app.devices.filter()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::tests::{at, connected_app, routed_row, row};

    /// Measuring a cell gives the width of the text the table draws.
    #[test]
    fn cell_widths_match_the_drawn_text() {
        let mut app = connected_app();
        app.now = at(4_000);
        let mut wide = row(4_194_303, "192.168.100.102:47808", 65_535, 0);
        wide.max_apdu = 50;
        let rows = [
            row(0, "10.0.0.1:47808", 0, 3_999),
            wide,
            routed_row(9, (65_534, &[1, 2, 3, 4, 5, 6]), "10.0.0.9:47808", 99, 10),
        ];
        let fields = [
            Field::Marker,
            Field::Instance,
            Field::Name,
            Field::Address,
            Field::Via,
            Field::Network,
            Field::Vendor,
            Field::Apdu,
            Field::Segmentation,
            Field::Seen,
        ];
        for row in &rows {
            for field in fields {
                let text = cell(field, row, &app, false);
                assert_eq!(
                    cell_width(field, row, &app),
                    text_width(&text),
                    "{field:?} {text:?}"
                );
            }
        }
    }
}
