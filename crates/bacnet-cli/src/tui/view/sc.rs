//! The SC screen: the hub connector and identity panels on top, the hub's
//! certificate chain, the connection doctor or this node's own credentials
//! in the middle (Tab switches), and the event timeline below. Every state
//! is marked by text as well as colour.
//!
//! Panes fit their content (#1890): the top row splits where its panels
//! wrap least, the middle takes what its view needs, and the timeline gets
//! the rest. From [`WIDE_WIDTH`] columns the credentials sit beside the
//! hub's chain or the doctor.

mod layout;

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use bacnet_transport::sc::ScConnectionState;

use super::{Theme, WIDE_WIDTH};
use crate::tui::app::App;
use crate::tui::app::devices::age_label;
use crate::tui::app::sc::{ScState, Severity, View};
use crate::tui::sc::credentials::signature_algorithm;
use crate::tui::sc::doctor::{CONNECTED, Diagnosis, Mark, PHASES};
use crate::tui::sc::{CertSummary, hub_name, uuid_text, vmac_text};
pub(super) use layout::Row;

/// The timeline's least height, borders included: one event line.
const EVENTS_MIN: u16 = 3;
/// Label width in the hub connector panel.
const STATUS_LABEL: usize = 11;
/// Label width in the identity panel.
const IDENTITY_LABEL: usize = 12;
/// Where a phase's text starts in the doctor: mark and label.
const PHASE_INDENT: usize = 17;
/// Where a certificate's details start in the chain.
const CERT_INDENT: usize = 10;
/// Where Hint, Detail and Hub sent text starts in the doctor.
const NOTE_INDENT: usize = 9;

/// The SC screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.transport != "SC" {
        let block = Block::bordered()
            .title(Line::styled(" BACnet/SC ", theme.accent()))
            .border_style(theme.dim());
        let text = "SC only: the hub connector belongs to BACnet/SC (Annex AB). Connect an SC \
                    profile (p) to use this screen.";
        let lines = Paragraph::new(Line::styled(text, theme.dim())).wrap(Wrap { trim: true });
        frame.render_widget(lines.block(block), area);
        return;
    }
    let status = status_rows(app, theme);
    let identity = identity_rows(&app.sc, app.now, theme);
    let (identity_width, top_height) = layout::split(area.width, &status, &identity);
    let top_height = top_height.min(area.height.saturating_sub(2 * EVENTS_MIN));
    let [top, rest] =
        Layout::vertical([Constraint::Length(top_height), Constraint::Min(0)]).areas(area);
    let [status_area, identity_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(identity_width)]).areas(top);
    panel(frame, status_area, " Hub connector ", &status, theme);
    panel(frame, identity_area, " VMAC and UUID ", &identity, theme);
    // The middle takes what its view needs; the timeline gets the rest.
    let room = rest.height.saturating_sub(EVENTS_MIN);
    let wide = area.width >= WIDE_WIDTH;
    let middle_height = if wide {
        wide_middle(frame, rest, room, app, theme)
    } else {
        narrow_middle(frame, rest, room, app, theme)
    };
    let [_, events] =
        Layout::vertical([Constraint::Length(middle_height), Constraint::Min(0)]).areas(rest);
    timeline(frame, events, app, theme);
}

/// The middle as one pane with three views; returns its height.
fn narrow_middle(frame: &mut Frame, rest: Rect, room: u16, app: &App, theme: &Theme) -> u16 {
    let inner = usize::from(rest.width.saturating_sub(2));
    let rows = view_rows(&app.sc, app.sc.view, inner, room, theme);
    let height = fit(&rows, inner, room);
    let area = Rect { height, ..rest };
    let names = [
        (View::Status, "Hub certificates"),
        (View::Doctor, "Doctor"),
        (View::Credentials, credentials_name(&app.sc)),
    ];
    let title = tabs(app, &names, app.sc.view, theme);
    framed(frame, area, title, &rows, theme);
    height
}

/// The middle at [`WIDE_WIDTH`] and up: the chain or the doctor, with the
/// credentials beside it; returns its height.
fn wide_middle(frame: &mut Frame, rest: Rect, room: u16, app: &App, theme: &Theme) -> u16 {
    let sc = &app.sc;
    // Tab still steps through all three; on the credentials, the chain shows.
    let left_view = match sc.view {
        View::Doctor => View::Doctor,
        View::Status | View::Credentials => View::Status,
    };
    let credentials = super::sc_credentials::rows(sc.credentials.as_ref(), theme);
    let natural = |rows: &[Row]| u16::try_from(layout::natural(rows) + 2).unwrap_or(u16::MAX);
    let half = rest.width / 2;
    let probe = view_rows(sc, left_view, usize::from(half), room, theme);
    let left_width = match (natural(&probe), natural(&credentials)) {
        (left, _) if left < half => left,
        (_, right) if right < half => rest.width - right,
        _ => half,
    };
    let left_inner = usize::from(left_width.saturating_sub(2));
    let right_inner = usize::from((rest.width - left_width).saturating_sub(2));
    let left = view_rows(sc, left_view, left_inner, room, theme);
    let height = fit(&left, left_inner, room).max(fit(&credentials, right_inner, room));
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(0)])
            .areas(Rect { height, ..rest });
    let names = [(View::Status, "Hub certificates"), (View::Doctor, "Doctor")];
    let selected = if sc.view == View::Credentials {
        None
    } else {
        Some(sc.view)
    };
    let title = tabs(app, &names, selected.unwrap_or(View::Credentials), theme);
    framed(frame, left_area, title, &left, theme);
    let name = credentials_name(sc);
    let title = if selected.is_none() {
        Line::styled(format!(" [{name}] "), theme.accent())
    } else {
        Line::styled(format!(" {name} "), theme.dim())
    };
    framed(frame, right_area, title, &credentials, theme);
    height
}

/// The height `rows` need at `inner` columns, borders included, within `room`.
fn fit(rows: &[Row], inner: usize, room: u16) -> u16 {
    let lines = u16::try_from(layout::height(rows, inner)).unwrap_or(u16::MAX);
    lines.saturating_add(2).min(room)
}

/// A view's rows at `inner` columns. The doctor folds the phases it passed
/// or never reached into one line each when its whole reading would not
/// fit in `room`, so the error text stays on screen.
fn view_rows<'a>(sc: &ScState, view: View, inner: usize, room: u16, theme: &Theme) -> Vec<Row<'a>> {
    match view {
        View::Status => chain(&sc.chain, theme),
        View::Credentials => super::sc_credentials::rows(sc.credentials.as_ref(), theme),
        View::Doctor => {
            let full = doctor(sc, false, theme);
            if layout::height(&full, inner) + 2 > usize::from(room) {
                doctor(sc, true, theme)
            } else {
                full
            }
        }
    }
}

fn credentials_name(sc: &ScState) -> &'static str {
    if sc.lint_refused() {
        "My credentials (FAIL)"
    } else {
        "My credentials"
    }
}

/// A bordered pane of `rows`, wrapped to its width. Lines past its height
/// give way to a `+N more` line, so a cut pane says so.
fn framed(frame: &mut Frame, area: Rect, title: Line, rows: &[Row], theme: &Theme) {
    let block = Block::bordered().title(title).border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = layout::wrap_all(rows, usize::from(inner.width));
    let height = usize::from(inner.height);
    if lines.len() > height && height > 0 {
        let hidden = lines.len() - (height - 1);
        lines.truncate(height - 1);
        lines.push(Line::styled(format!("+{hidden} more"), theme.warn()));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn panel(frame: &mut Frame, area: Rect, title: &str, rows: &[Row], theme: &Theme) {
    let title = Line::styled(title.to_string(), theme.accent());
    framed(frame, area, title, rows, theme);
}

fn tabs<'a>(app: &App, names: &[(View, &str)], selected: View, theme: &Theme) -> Line<'a> {
    let sc = &app.sc;
    let mut spans = vec![Span::raw(" ")];
    for (view, name) in names {
        if *view == selected {
            spans.push(Span::styled(format!("[{name}]"), theme.accent()));
        } else {
            spans.push(Span::styled(name.to_string(), theme.dim()));
        }
        spans.push(Span::raw(" "));
    }
    let note = match &sc.failure {
        Some(last) => format!(
            "| last failure {} ago{} ",
            age_label(app.now, last.at),
            last.hub
                .map_or_else(String::new, |hub| format!(", {} hub", hub_name(hub)))
        ),
        None => "| no failure recorded ".into(),
    };
    spans.push(Span::styled(note, theme.dim()));
    Line::from(spans)
}

/// A labelled field whose value wraps under itself.
fn field<'a>(label: &str, width: usize, value: String, style: Style, theme: &Theme) -> Row<'a> {
    let line = Line::from(vec![
        Span::styled(format!("{label:<width$}"), theme.dim()),
        Span::styled(value, style),
    ]);
    Row::hang(line, width)
}

fn state_word(state: ScConnectionState) -> &'static str {
    match state {
        ScConnectionState::Disconnected => "disconnected",
        ScConnectionState::Connecting => "connecting",
        ScConnectionState::Connected => "connected",
        ScConnectionState::Disconnecting => "disconnecting",
    }
}

/// The hub connector panel. The hub's VMAC is in the identity panel.
fn status_rows<'a>(app: &App, theme: &Theme) -> Vec<Row<'a>> {
    let sc = &app.sc;
    let Some((link, _)) = &sc.link else {
        let text = match &app.sc.failure {
            Some(_) => "Not connected: the doctor (Tab) shows where the dial failed.",
            None => "Waiting for the hub connector's first report.",
        };
        return vec![Row::flow(Line::styled(text, theme.dim()))];
    };
    let style = match link.state {
        ScConnectionState::Connected => theme.accent(),
        ScConnectionState::Connecting => theme.warn(),
        _ => theme.danger(),
    };
    let url = |hub| {
        sc.identity
            .as_ref()
            .and_then(|identity| identity.url(hub))
            .unwrap_or("")
            .to_string()
    };
    // The hub's name goes with the state, so the URL has its line to itself.
    let (state, hub) = match link.hub {
        Some(hub) => (
            format!("{}, {} hub", state_word(link.state), hub_name(hub)),
            url(hub),
        ),
        None => (state_word(link.state).into(), "none dialled yet".into()),
    };
    let standby = match sc.identity.as_ref().and_then(|i| i.failover.as_ref()) {
        Some(url) => url.clone(),
        None => "none configured".into(),
    };
    let attempt = match sc.attempt {
        Some((hub, attempt, backoff)) => format!(
            "attempt {attempt} to the {} hub after {} ms backoff",
            hub_name(hub),
            backoff.as_millis()
        ),
        None => "none since the last connect".into(),
    };
    let heartbeat = match (sc.heartbeat_age(app.now), link.heartbeat_rtt) {
        (Some(age), Some(rtt)) => format!(
            "heard {} s ago, round trip {} ms",
            age.as_secs(),
            rtt.as_millis()
        ),
        (Some(age), None) => format!("heard {} s ago, no heartbeat answered yet", age.as_secs()),
        (None, _) => "not connected".into(),
    };
    let w = STATUS_LABEL;
    vec![
        field("State", w, state, style, theme),
        field("Hub", w, hub, theme.accent(), theme),
        field("Reconnect", w, attempt, Style::new(), theme),
        field("Heartbeat", w, heartbeat, Style::new(), theme),
        field("Failover", w, standby, theme.dim(), theme),
    ]
}

fn identity_rows<'a>(sc: &ScState, now: Instant, theme: &Theme) -> Vec<Row<'a>> {
    let (vmac, uuid) = match &sc.identity {
        Some(identity) => (vmac_text(&identity.vmac), uuid_text(&identity.uuid)),
        None => ("unknown".into(), "unknown".into()),
    };
    let hub_vmac = sc
        .link
        .as_ref()
        .and_then(|(link, _)| link.hub_vmac)
        .map_or("unknown".into(), |v| vmac_text(&v));
    let w = IDENTITY_LABEL;
    let mut rows = vec![
        field("Local VMAC", w, vmac, theme.accent(), theme),
        field("Device UUID", w, uuid, theme.accent(), theme),
        field("Hub VMAC", w, hub_vmac, theme.accent(), theme),
    ];
    match sc.duplicates.back() {
        None => rows.push(field(
            "Dup. VMAC",
            w,
            "no NAK seen".into(),
            theme.dim(),
            theme,
        )),
        Some(last) => {
            let count = format!(
                "{} NAK(s), last {} ago",
                sc.duplicates.len(),
                age_label(now, last.at)
            );
            rows.push(field("Dup. VMAC", w, count, theme.danger(), theme));
            let next = if last.reseeded {
                "a new random VMAC was drawn"
            } else {
                "VMAC kept"
            };
            rows.push(field("", w, next.into(), theme.dim(), theme));
        }
    }
    rows
}

fn certificate<'a>(index: usize, cert: &CertSummary, theme: &Theme) -> Vec<Row<'a>> {
    let role = if index == 0 { "leaf" } else { "issuer" };
    let detail = |text: String| Row::hang(format!("{:CERT_INDENT$}{text}", ""), CERT_INDENT);
    vec![
        Row::hang(
            Line::from(vec![
                Span::styled(format!("{index} {role:<8}"), theme.dim()),
                Span::styled(format!("subject {}", cert.subject), theme.accent()),
            ]),
            CERT_INDENT,
        ),
        detail(format!("issued by {}", cert.issuer)),
        detail(format!("expires {}", cert.not_after)),
        detail(format!(
            "signed with {}",
            signature_algorithm(&cert.signature_algorithm)
        )),
    ]
}

fn chain<'a>(chain: &[CertSummary], theme: &Theme) -> Vec<Row<'a>> {
    if chain.is_empty() {
        return vec![Row::flow(Line::styled(
            "No hub certificate: not connected to a hub.",
            theme.dim(),
        ))];
    }
    let mut rows = vec![Row::flow(Line::styled(
        "As the connected hub presented them, leaf first. Display only: TLS verified the chain.",
        theme.dim(),
    ))];
    for (index, cert) in chain.iter().enumerate() {
        rows.extend(certificate(index, cert, theme));
    }
    rows
}

fn mark<'a>(mark: Mark, theme: &Theme) -> Span<'a> {
    match mark {
        Mark::Passed => Span::styled("[ ok ] ", theme.accent()),
        Mark::Failed => Span::styled("[FAIL] ", theme.danger()),
        Mark::NotReached => Span::styled("[ -- ] ", theme.dim()),
        Mark::Unknown => Span::styled("[ ?? ] ", theme.warn()),
    }
}

/// One row per phase; `compact` folds runs of phases with the same mark,
/// other than the failed one, into one row naming them.
fn phases<'a>(
    marks: &[Mark; 6],
    failed: Option<&Diagnosis>,
    compact: bool,
    theme: &Theme,
) -> Vec<Row<'a>> {
    let mut rows: Vec<Row> = Vec::new();
    let mut run: Option<(Mark, Vec<&str>)> = None;
    let flush = |run: &mut Option<(Mark, Vec<&str>)>, rows: &mut Vec<Row<'a>>| {
        if let Some((m, names)) = run.take() {
            let line = Line::from(vec![
                mark(m, theme),
                Span::styled(names.join(", "), theme.accent()),
            ]);
            rows.push(Row::hang(line, 7));
        }
    };
    for (phase, m) in PHASES.iter().zip(marks) {
        let failing = failed.filter(|d| d.failed == Some(*phase));
        if compact && failing.is_none() {
            match &mut run {
                Some((last, names)) if last == m => names.push(phase.label()),
                _ => {
                    flush(&mut run, &mut rows);
                    run = Some((*m, vec![phase.label()]));
                }
            }
            continue;
        }
        flush(&mut run, &mut rows);
        let what = match failing {
            Some(d) => Span::styled(d.summary.clone(), theme.danger()),
            None => Span::styled(phase.what(), theme.dim()),
        };
        let line = Line::from(vec![
            mark(*m, theme),
            Span::styled(format!("{:<10}", phase.label()), theme.accent()),
            what,
        ]);
        rows.push(Row::hang(line, PHASE_INDENT));
    }
    flush(&mut run, &mut rows);
    rows
}

fn doctor<'a>(sc: &ScState, compact: bool, theme: &Theme) -> Vec<Row<'a>> {
    let connected = sc
        .link
        .as_ref()
        .is_some_and(|(link, _)| link.state == ScConnectionState::Connected);
    let Some(last) = &sc.failure else {
        if connected {
            let mut rows = phases(&CONNECTED, None, compact, theme);
            rows.push(Row::flow(""));
            rows.push(Row::flow(Line::styled(
                "Connected: every phase passed.",
                theme.accent(),
            )));
            return rows;
        }
        return vec![Row::flow(Line::styled(
            "No failure recorded. A failed connect shows here, phase by phase.",
            theme.dim(),
        ))];
    };
    let d = &last.diagnosis;
    let mut rows = phases(&d.marks, Some(d), compact, theme);
    if d.failed.is_none() {
        rows.push(Row::flow(Line::styled(
            format!("Not tied to one phase: {}", d.summary),
            theme.warn(),
        )));
    }
    rows.push(Row::flow(""));
    let note = |label: &str, style: Style, text: Span<'a>| {
        let label = Span::styled(format!("{label:<NOTE_INDENT$}"), style);
        Row::hang(Line::from(vec![label, text]), NOTE_INDENT)
    };
    rows.push(note("Hint", theme.warn(), Span::raw(d.hint.clone())));
    // The error in full: the footer has room for a line only.
    let detail = Span::styled(last.failure.detail.clone(), theme.dim());
    rows.push(note("Detail", theme.dim(), detail));
    if let Some(leaf) = last.failure.presented.first() {
        rows.push(note(
            "Hub sent",
            theme.dim(),
            Span::raw(format!(
                "subject {}, issued by {}, expires {}, signed with {}",
                leaf.subject,
                leaf.issuer,
                leaf.not_after,
                signature_algorithm(&leaf.signature_algorithm)
            )),
        ));
    }
    if connected {
        rows.push(Row::flow(Line::styled(
            "Connected again since: every phase passes now.",
            theme.accent(),
        )));
    }
    rows
}

fn timeline(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let sc = &app.sc;
    let inner = Block::bordered().inner(area);
    let width = usize::from(inner.width);
    let rows: Vec<Row> = if sc.timeline.is_empty() {
        vec![Row::flow(Line::styled(
            "Connects, losses, reconnect attempts, failover and the primary's return show here.",
            theme.dim(),
        ))]
    } else {
        sc.timeline
            .iter()
            .map(|entry| {
                let level = match entry.severity {
                    Severity::Info => Span::raw("info "),
                    Severity::Warn => Span::styled("warn ", theme.warn()),
                    Severity::Error => Span::styled("error", theme.danger()),
                };
                let age = format!("{:>5} ago ", age_label(app.now, entry.at));
                // The text hangs after the age and the level, however wide.
                let indent = layout::cols(&age) + level.width() + 1;
                let mut spans = vec![
                    Span::styled(age, theme.dim()),
                    level,
                    Span::raw(format!(" {}", entry.text)),
                ];
                if entry.repeats > 1 {
                    spans.push(Span::styled(
                        format!(" ({} times)", entry.repeats),
                        theme.dim(),
                    ));
                }
                let line = Line::from(spans);
                Row::hang(line, indent)
            })
            .collect()
    };
    // The newest events that fit, each wrapped whole; the title counts the
    // older ones that don't, so a cut timeline costs no line to say so.
    let (lines, shown) = newest(&rows, width, usize::from(inner.height));
    let mut title = vec![Span::styled(" Events ", theme.accent())];
    if sc.missed > 0 {
        title.push(Span::styled(
            format!("({} missed) ", sc.missed),
            theme.accent(),
        ));
    }
    if shown < rows.len() {
        let earlier = format!("+{} earlier ", rows.len() - shown);
        title.push(Span::styled(earlier, theme.warn()));
    }
    let block = Block::bordered()
        .title(Line::from(title))
        .border_style(theme.dim());
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The newest `rows` that fit in `height` lines, each wrapped whole, oldest
/// first, and how many there are.
fn newest<'a>(rows: &[Row<'a>], width: usize, height: usize) -> (Vec<Line<'a>>, usize) {
    let mut lines: Vec<Line> = Vec::new();
    let mut shown = 0;
    for row in rows.iter().rev() {
        let wrapped = layout::wrap(row, width);
        if !lines.is_empty() && lines.len() + wrapped.len() > height {
            break;
        }
        lines.splice(0..0, wrapped);
        shown += 1;
    }
    (lines, shown)
}
