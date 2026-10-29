//! The watch screen: one row per watched point, with its latest value,
//! where values come from, their age and a sparkline drawn with the
//! eighth-block bar symbols (never Braille, which many terminal fonts lack).

use std::collections::VecDeque;

use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::limits::METER_WINDOW_SECS;
use bacnet_types::enums::PropertyIdentifier;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::symbols::bar;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table};

use super::{FULL_WIDTH, Theme, WIDE_WIDTH, ellipsize as fit, sanitize, text_width};
use crate::tui::app::App;
use crate::tui::app::devices::age_label;
use crate::tui::app::watch::WatchRow;
use crate::tui::watched::Source;

/// The bar levels, lowest first.
const LEVELS: [&str; 8] = [
    bar::ONE_EIGHTH,
    bar::ONE_QUARTER,
    bar::THREE_EIGHTHS,
    bar::HALF,
    bar::FIVE_EIGHTHS,
    bar::THREE_QUARTERS,
    bar::SEVEN_EIGHTHS,
    bar::FULL,
];

/// The newest `width` values as bars scaled between their minimum and
/// maximum; a flat run sits at half height.
pub(crate) fn sparkline(values: &VecDeque<f64>, width: usize) -> String {
    let shown: Vec<f64> = values
        .iter()
        .skip(values.len().saturating_sub(width))
        .copied()
        .collect();
    let (low, high) = shown
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    let span = high - low;
    shown
        .iter()
        .map(|v| {
            if span <= f64::EPSILON * high.abs().max(1.0) {
                return LEVELS[3];
            }
            let level = ((v - low) / span * 7.0).round() as usize;
            LEVELS[level.min(7)]
        })
        .collect()
}

fn source_label(source: &Source) -> &'static str {
    match source {
        Source::Starting => "start",
        Source::Cov { .. } => "COV",
        Source::Polled(_) => "polled",
        Source::Failed(_) => "failed",
    }
}

fn title(app: &App) -> String {
    let stats = &app.watch.stats;
    let rate = stats.recent_requests as f64 / METER_WINDOW_SECS as f64;
    format!(
        " Watch ({}) | COV {} | requests {rate:.1}/s ",
        app.watch.rows().len(),
        stats.subscriptions
    )
}

/// The three layouts: under [`FULL_WIDTH`], up to [`WIDE_WIDTH`], and wider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Narrow,
    Full,
    Wide,
}

/// The most a name, a value and the units may take, and the least the
/// trend keeps.
struct Caps {
    name: usize,
    value: usize,
    units: usize,
    trend: usize,
}

impl Tier {
    fn of(width: u16) -> Self {
        if width >= WIDE_WIDTH {
            Self::Wide
        } else if width >= FULL_WIDTH {
            Self::Full
        } else {
            Self::Narrow
        }
    }

    fn caps(self) -> Caps {
        match self {
            Self::Narrow => Caps {
                name: 24,
                value: 18,
                units: 0,
                trend: 8,
            },
            Self::Full => Caps {
                name: 28,
                value: 20,
                units: 0,
                trend: 12,
            },
            Self::Wide => Caps {
                name: 40,
                value: 28,
                units: 24,
                trend: 16,
            },
        }
    }
}

/// The columns before the trend. Device and Object identify a row and are
/// never cut; Name takes what the others leave, down to [`MIN_NAME`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Device,
    Object,
    Name,
    Property,
    Value,
    Units,
    Flags,
    Source,
    Age,
}

/// Every column, in order.
const COLUMNS: [Column; 9] = [
    Column::Device,
    Column::Object,
    Column::Name,
    Column::Property,
    Column::Value,
    Column::Units,
    Column::Flags,
    Column::Source,
    Column::Age,
];

/// The narrowest name column worth showing; below it the name shows only
/// in the line under the table.
const MIN_NAME: usize = 6;

impl Column {
    fn header(self) -> &'static str {
        match self {
            Self::Device => "Device",
            Self::Object => "Object",
            Self::Name => "Name",
            Self::Property => "Property",
            Self::Value => "Value",
            Self::Units => "Units",
            Self::Flags => "Flags",
            Self::Source => "Source",
            Self::Age => "Age",
        }
    }

    /// Property from [`FULL_WIDTH`], and below it only when some row
    /// watches another property than Present_Value (`not_pv`), so two rows
    /// of one object stay apart. Units only from [`WIDE_WIDTH`]: the value
    /// carries the unit symbol, so a name gets the room first.
    fn shown(self, tier: Tier, not_pv: bool) -> bool {
        match self {
            Self::Property => tier != Tier::Narrow || not_pv,
            Self::Units => tier == Tier::Wide,
            _ => true,
        }
    }

    /// The most this column may take; `None` fits its content.
    fn cap(self, caps: &Caps) -> Option<usize> {
        match self {
            Self::Name => Some(caps.name),
            Self::Value => Some(caps.value),
            Self::Units => Some(caps.units),
            _ => None,
        }
    }
}

/// Status_Flags as text badges without brackets, to fit the column:
/// `ALM FLT OVR OOS`.
fn flags_text(flags: Option<crate::tui::point::Health>) -> String {
    flags
        .map(|f| {
            f.badges()
                .iter()
                .map(|badge| badge.trim_matches(['[', ']']))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// A row's text in `column`, before fitting.
fn cell(row: &WatchRow, app: &App, column: Column) -> String {
    match column {
        Column::Device => row.point.device_label(),
        Column::Object => object_label(row.point.object),
        // Sanitized when it was read.
        Column::Name => row.name.clone().unwrap_or_default(),
        Column::Property => kebab(row.point.property),
        Column::Value => match &row.value {
            Some(Ok(value)) => sanitize(value),
            Some(Err(error)) => format!("({})", sanitize(error)),
            None => "-".into(),
        },
        Column::Units => row.units.clone().unwrap_or_default(),
        Column::Flags => flags_text(row.flags),
        Column::Source => source_label(&row.source).to_string(),
        Column::Age => row
            .updated
            .map_or_else(|| "-".into(), |at| age_label(app.now, at)),
    }
}

/// The narrowest a value is cut to before columns go.
const MIN_VALUE: usize = 8;

/// The columns a table `inner` columns wide shows, each with its width,
/// and the trend's width. `texts` holds each row's cells in [`COLUMNS`]
/// order. Every column fits its content and header (Value and Units up to
/// their caps); Name, shown only when some row has one, takes what is left
/// after the trend's least, up to its cap; the trend takes the rest.
///
/// When even that doesn't fit, room comes from the least telling columns
/// first: Value down to [`MIN_VALUE`], then Age and Source go (the line
/// under the table names the source), then Flags is cut to its header,
/// then the trend to one column. Device and Object are never cut.
fn layout(texts: &[Vec<String>], inner: u16, tier: Tier) -> (Vec<(Column, usize)>, usize) {
    let caps = tier.caps();
    let property = COLUMNS.iter().position(|c| *c == Column::Property);
    let not_pv = texts
        .iter()
        .any(|row| property.is_some_and(|at| row[at] != "present-value"));
    let mut columns: Vec<(Column, usize)> = Vec::new();
    let mut name = None;
    for (i, column) in COLUMNS.into_iter().enumerate() {
        if !column.shown(tier, not_pv) {
            continue;
        }
        let widest = texts.iter().map(|row| text_width(&row[i])).max();
        let widest = widest.unwrap_or(0);
        if column == Column::Name {
            if widest > 0 {
                name = Some((columns.len(), widest.max(column.header().len())));
            }
            continue;
        }
        let width = widest.max(column.header().len());
        let width = column.cap(&caps).map_or(width, |cap| width.min(cap));
        columns.push((column, width));
    }
    let inner = usize::from(inner);
    let used = |columns: &[(Column, usize)]| columns.iter().map(|(_, w)| w + 1).sum::<usize>();
    let mut trend = caps.trend;
    let over = |columns: &[(Column, usize)], trend: usize| used(columns) + trend > inner;
    if over(&columns, trend) {
        let excess = used(&columns) + trend - inner;
        if let Some((_, width)) = columns.iter_mut().find(|(c, _)| *c == Column::Value) {
            *width = width.saturating_sub(excess).max(MIN_VALUE).min(*width);
        }
    }
    for gone in [Column::Age, Column::Source] {
        if over(&columns, trend) {
            columns.retain(|(c, _)| *c != gone);
        }
    }
    if over(&columns, trend) {
        let excess = used(&columns) + trend - inner;
        if let Some((column, width)) = columns.iter_mut().find(|(c, _)| *c == Column::Flags) {
            *width = width.saturating_sub(excess).max(column.header().len());
        }
    }
    if over(&columns, trend) {
        trend = inner.saturating_sub(used(&columns)).max(1);
    }
    let mut left = inner.saturating_sub(used(&columns));
    if let Some((at, widest)) = name {
        let room = left.saturating_sub(trend + 1);
        let width = widest.min(caps.name).min(room);
        if width >= MIN_NAME {
            columns.insert(at, (Column::Name, width));
            left -= width + 1;
        }
    }
    (columns, left.max(1))
}

/// The line under the table for the selected `row`, `width` columns wide:
/// where its values come from (`source`) first and whole, then the row
/// itself (object, a property other than Present_Value, the whole name),
/// cut to the room left.
fn detail_line(row: &WatchRow, source: String, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    let mut about = object_label(row.point.object);
    if row.point.property != PropertyIdentifier::PRESENT_VALUE {
        about = format!("{about} {}", kebab(row.point.property));
    }
    let source = format!(" {source}");
    let room = width.saturating_sub(text_width(&source) + 3);
    // The name gets what the object and property leave.
    if let Some(name) = &row.name {
        let left = room.saturating_sub(text_width(&about) + 2);
        if left >= MIN_NAME {
            about = format!("{about}, {}", fit(name, left));
        }
    }
    let mut spans = vec![Span::styled(source, theme.dim())];
    if room >= MIN_NAME {
        spans.push(Span::styled(" · ", theme.dim()));
        spans.push(Span::styled(fit(&about, room), theme.accent()));
    }
    spans
}

/// The watch screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut block = Block::bordered()
        .title(Line::styled(title(app), theme.accent()))
        .border_style(theme.dim());
    if let Some(path) = &app.watch.stats.csv {
        block = block.title(
            Line::styled(format!(" csv: {path} "), theme.warn()).alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = app.watch.rows();
    if rows.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled("No points watched.", theme.dim()),
            Line::styled(
                "In the browser, select an object and press a; or type :watch DEVICE OBJECT [PROPERTY].",
                theme.dim(),
            ),
        ];
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
        return;
    }
    let [table_area, detail] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(1)]).areas(inner);
    let texts: Vec<Vec<String>> = rows
        .iter()
        .map(|row| COLUMNS.iter().map(|c| cell(row, app, *c)).collect())
        .collect();
    let (columns, spark) = layout(&texts, table_area.width, Tier::of(area.width));
    let length = |width: usize| Constraint::Length(u16::try_from(width).unwrap_or(u16::MAX));
    let mut widths: Vec<Constraint> = columns.iter().map(|(_, w)| length(*w)).collect();
    widths.push(length(spark));
    let mut header: Vec<&str> = columns.iter().map(|(c, _)| c.header()).collect();
    header.push("Trend");

    let height = usize::from(table_area.height.saturating_sub(1)).max(1);
    let selected = app.watch.selected();
    let offset = selected.saturating_sub(height - 1);
    let lines: Vec<Row> = rows
        .iter()
        .zip(&texts)
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, (row, text))| {
            let mut cells: Vec<String> = columns
                .iter()
                .map(|(column, width)| {
                    let at = COLUMNS.iter().position(|c| c == column).unwrap_or(0);
                    fit(&text[at], *width)
                })
                .collect();
            cells.push(sparkline(&row.history, spark));
            let line = Row::new(cells);
            if i == selected {
                line.style(theme.selected())
            } else {
                line
            }
        })
        .collect();
    let table = Table::new(lines, widths)
        .header(Row::new(header).style(theme.accent()))
        .column_spacing(1);
    frame.render_widget(table, table_area);

    if let Some(row) = rows.get(selected) {
        let text = match &row.source {
            Source::Starting => "reading and subscribing".to_string(),
            Source::Cov { lifetime } => {
                format!("COV, lifetime {lifetime} s, renewed before it runs out")
            }
            Source::Polled(reason) => format!("polled: {reason}"),
            Source::Failed(error) => error.clone(),
        };
        let line = Line::from(detail_line(row, text, usize::from(detail.width), theme));
        frame.render_widget(Paragraph::new(line), detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sparkline_scales_between_the_extremes_and_keeps_the_newest() {
        let values: VecDeque<f64> = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0].into();
        assert_eq!(sparkline(&values, 8), "▁▂▃▄▅▆▇█");
        let flat: VecDeque<f64> = [5.0; 4].into();
        assert_eq!(sparkline(&flat, 10), "▄▄▄▄");
        assert_eq!(sparkline(&VecDeque::new(), 5), "");
    }

    #[test]
    fn fit_cuts_with_an_ellipsis_by_display_width() {
        assert_eq!(fit("Supply Fan", 10), "Supply Fan");
        assert_eq!(fit("Supply Fan", 7), "Supply…");
        // Each of these is two columns wide.
        assert_eq!(fit("温度センサー", 5), "温度…");
        assert_eq!(fit("abc", 0), "");
    }

    fn texts(device: &str, object: &str, name: &str) -> Vec<String> {
        let mut row = vec![String::new(); COLUMNS.len()];
        row[0] = device.into();
        row[1] = object.into();
        row[2] = name.into();
        row[3] = "present-value".into();
        row[4] = "72.5 °F".into();
        row
    }

    #[test]
    fn identifying_columns_fit_their_content_and_the_name_takes_the_rest() {
        let rows = [
            texts(
                "2:2001",
                "multi-state-output 4194302",
                "Discharge Air Temperature",
            ),
            texts("1", "ai 1", ""),
        ];
        let (columns, trend) = layout(&rows, 78, Tier::Narrow);
        let width = |c| columns.iter().find(|(col, _)| *col == c).map(|(_, w)| *w);
        assert_eq!(width(Column::Device), Some(6));
        assert_eq!(width(Column::Object), Some(26));
        assert_eq!(width(Column::Property), None);
        assert_eq!(width(Column::Units), None);
        assert_eq!(trend, Tier::Narrow.caps().trend);
        let used: usize = columns.iter().map(|(_, w)| w + 1).sum();
        assert_eq!(used + trend, 78);
        // No row has a name: no Name column, and the trend has the room.
        let rows = [texts("1", "ai 1", "")];
        let (columns, _) = layout(&rows, 78, Tier::Narrow);
        assert!(columns.iter().all(|(c, _)| *c != Column::Name));
        // Too little room: the name shows only under the table.
        let rows = [texts("4194302", "multi-state-output 4194302", "Discharge")];
        let (columns, _) = layout(&rows, 60, Tier::Narrow);
        assert!(columns.iter().all(|(c, _)| *c != Column::Name));
        assert!(columns.contains(&(Column::Object, 26)));
    }

    #[test]
    fn a_row_too_wide_gives_up_value_then_age_and_source_before_identity() {
        let mut widest = texts("4194302", "multi-state-output 4194302", "");
        widest[4] = "12 (Economizer Minimum Position)".into();
        widest[6] = "ALM FLT OVR OOS".into();
        widest[7] = "COV".into();
        widest[8] = "59m".into();
        let (columns, trend) = layout(std::slice::from_ref(&widest), 78, Tier::Narrow);
        let used: usize = columns.iter().map(|(_, w)| w + 1).sum();
        assert!(used + trend <= 78, "{columns:?} {trend}");
        assert!(columns.contains(&(Column::Device, 7)));
        assert!(columns.contains(&(Column::Object, 26)));
        assert!(columns.contains(&(Column::Value, MIN_VALUE)));
        assert!(columns.contains(&(Column::Flags, 15)));
        assert!(columns.iter().all(|(c, _)| *c != Column::Age));
        // Narrower still: Source goes, then Flags is cut, then the trend.
        let (columns, trend) = layout(&[widest], 60, Tier::Narrow);
        let used: usize = columns.iter().map(|(_, w)| w + 1).sum();
        assert!(used + trend <= 60, "{columns:?} {trend}");
        assert!(columns.contains(&(Column::Device, 7)));
        assert!(columns.contains(&(Column::Object, 26)));
        assert!(columns.iter().all(|(c, _)| *c != Column::Source));
    }

    #[test]
    fn below_full_width_the_property_shows_only_when_it_tells_rows_apart() {
        let pv = texts("1", "ai 1", "");
        let mut flags = pv.clone();
        flags[3] = "status-flags".into();
        let shows = |rows: &[Vec<String>]| {
            let (columns, _) = layout(rows, 78, Tier::Narrow);
            columns.iter().any(|(c, _)| *c == Column::Property)
        };
        assert!(!shows(std::slice::from_ref(&pv)));
        assert!(shows(&[pv, flags]));
    }
}
