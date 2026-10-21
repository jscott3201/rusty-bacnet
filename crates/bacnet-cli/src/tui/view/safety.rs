//! Write session dialogs: arming, the write form, the preview with its
//! confirmation tier, the result, the ledger and the exit guard.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::{centered, Theme};
use crate::tui::app::objects::object_label;
use crate::tui::app::safety::{Form, Modal};
use crate::tui::app::{App, Link};
use crate::tui::safety::ledger::{Entry, State};
use crate::tui::safety::{Effect, Gate, Occupancy, Outcome, Planned, Preview, Tier};

/// Draw `modal` over the screen.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, modal: &Modal, theme: &Theme) {
    let (title, lines) = match modal {
        Modal::Arm(form) => (" Arm a write session ".to_string(), arm(app, form, theme)),
        Modal::Write {
            device,
            object,
            relinquish,
            form,
        } => {
            let what = if *relinquish { "Relinquish" } else { "Write" };
            let title = format!(" {what}: device {device} {} ", object_label(*object));
            (title, write_form(*relinquish, form, theme))
        }
        Modal::Waiting { what, .. } => (
            " Write session ".to_string(),
            vec![
                Line::raw(*what),
                Line::styled("Esc closes this view.", theme.dim()),
            ],
        ),
        Modal::Confirm {
            preview,
            typed,
            error,
            ..
        } => (
            format!(" Confirm ({}) ", tier_name(&preview.gate)),
            confirm(preview, typed, error.as_deref(), theme),
        ),
        Modal::Result(outcomes) => (" Result ".to_string(), result(outcomes, theme)),
        Modal::Ledger { notice, confirm } => (
            format!(" Session ledger ({}) ", app.safety.ledger.len()),
            ledger(app, notice.as_deref(), *confirm, theme),
        ),
        Modal::ExitGuard(error) => (
            " Changes still in effect ".to_string(),
            exit_guard(app, error.as_deref(), theme),
        ),
    };
    let width = area.width.saturating_sub(4).min(92);
    // Borders and padding take four columns; long lines wrap.
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    let height = (rows as u16 + 2).min(area.height);
    let box_area = centered(area, width, height);
    frame.render_widget(Clear, box_area);
    let block = Block::bordered()
        .title(Line::styled(title, theme.accent()))
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        box_area,
    );
}

fn fields<'a>(form: &'a Form, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = form
        .fields
        .iter()
        .enumerate()
        .map(|(i, (label, text))| {
            if i == form.focus {
                Line::styled(format!("> {label:<9} {text}_"), theme.selected())
            } else {
                Line::raw(format!("  {label:<9} {text}"))
            }
        })
        .collect();
    if let Some(error) = &form.error {
        lines.push(Line::styled(error.clone(), theme.danger()));
    }
    lines
}

fn arm<'a>(app: &App, form: &'a Form, theme: &Theme) -> Vec<Line<'a>> {
    let site = app
        .connection
        .as_ref()
        .and_then(|c| c.saved.as_ref().map(|_| c.name.clone()))
        .unwrap_or_else(|| "(no profile)".into());
    let network = match &app.link {
        Link::Up { local } => format!("{} {local}", app.transport),
        _ => app.transport.to_string(),
    };
    let mut lines = vec![
        Line::raw(format!("Site: {site}   Network: {network}")),
        Line::raw("Writes go to live equipment. Each one is previewed first,"),
        Line::raw("kept in the ledger (u) for revert, and written to audit.jsonl."),
        Line::raw(""),
    ];
    lines.extend(fields(form, theme));
    lines.push(Line::styled(
        "Reason: optional ticket or note. Enter arms | Tab next | Esc cancel",
        theme.dim(),
    ));
    lines
}

fn write_form<'a>(relinquish: bool, form: &'a Form, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = fields(form, theme);
    let hint = if relinquish {
        "Writes NULL at the slot: the next priority (or Relinquish_Default) takes over."
    } else {
        "Value as the one-shot write takes it: 42, 72.5, true, active, \"text\", null."
    };
    lines.push(Line::styled(hint, theme.dim()));
    lines.push(Line::styled(
        "Priority 1-16 (blank: not commandable). Enter previews | Esc cancel",
        theme.dim(),
    ));
    lines
}

fn tier_name(gate: &Gate) -> &'static str {
    match gate {
        Gate::Allowed(Tier::T1) => "T1",
        Gate::Allowed(Tier::T2) => "T2",
        Gate::Allowed(Tier::T3) => "T3",
        Gate::Blocked(_) => "blocked",
    }
}

fn item_lines(item: &Planned, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(item.label(), theme.accent())];
    let before = item.before.as_ref().map(|b| b.text.clone());
    if let Some(priority) = item.priority {
        let slot = match (item.occupancy, &before) {
            (Occupancy::Empty, _) => "empty".to_string(),
            (Occupancy::Ours, Some(text)) => format!("{text} (our earlier write)"),
            (_, Some(text)) => format!("{text} (another commander's)"),
            (_, None) => "?".into(),
        };
        lines.push(Line::raw(format!(
            "  slot {priority}: {slot} -> {}",
            item.value.text
        )));
    } else {
        lines.push(Line::raw(format!(
            "  value: {} -> {}",
            before.as_deref().unwrap_or("?"),
            item.value.text
        )));
    }
    match &item.effect {
        Effect::Changes { from, to } => lines.push(Line::styled(
            format!("  Present_Value will change: {from} -> {to}"),
            theme.warn(),
        )),
        Effect::Unchanged {
            held_by: Some(slot),
        } => lines.push(Line::raw(format!(
            "  Present_Value will not change: priority {slot} is in control"
        ))),
        Effect::Unchanged { held_by: None } => {
            lines.push(Line::raw("  Present_Value will not change"))
        }
        Effect::NotPresentValue => {}
    }
    if let Some(oos) = item.out_of_service {
        lines.push(Line::styled(
            format!("  Out_Of_Service: {oos}"),
            theme.dim(),
        ));
    }
    for note in &item.notes {
        lines.push(Line::styled(format!("  ! {note}"), theme.warn()));
    }
    lines
}

fn confirm(
    preview: &Preview,
    typed: &str,
    error: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if preview.batch {
        lines.push(Line::raw(format!(
            "One WritePropertyMultiple of {} write(s) to device {}:",
            preview.items.len(),
            preview.device
        )));
    }
    for item in &preview.items {
        lines.extend(item_lines(item, theme));
    }
    lines.push(Line::raw(""));
    match &preview.gate {
        Gate::Allowed(Tier::T1) => lines.push(Line::styled(
            "T1: Enter sends it | Esc cancels",
            theme.accent(),
        )),
        Gate::Allowed(Tier::T2) => lines.push(Line::styled(
            "T2: press y to send it | Esc cancels",
            theme.warn(),
        )),
        Gate::Allowed(Tier::T3) => {
            lines.push(Line::styled(
                "T3: this commands a life-safety, critical or reserved priority.",
                theme.danger(),
            ));
            lines.push(Line::raw(format!(
                "Type the device instance ({}) and Enter to send: {typed}_",
                preview.device
            )));
        }
        Gate::Blocked(why) => {
            lines.push(Line::styled(format!("BLOCKED: {why}"), theme.danger()));
            lines.push(Line::raw("Esc closes."));
        }
    }
    if let Some(error) = error {
        lines.push(Line::styled(error.to_string(), theme.danger()));
    }
    lines
}

fn result(outcomes: &[(String, Outcome)], theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (label, outcome) in outcomes {
        let style = match outcome {
            Outcome::Ack => theme.accent(),
            Outcome::Denied => theme.danger(),
            _ => theme.warn(),
        };
        lines.push(Line::raw(label.clone()));
        lines.push(Line::styled(format!("  {}", outcome.describe()), style));
    }
    lines.push(Line::styled("Enter or Esc closes.", theme.dim()));
    lines
}

fn entry_line(app: &App, entry: &Entry, theme: &Theme) -> Line<'static> {
    let undo = entry.undo().map_or("?".into(), |u| u.text);
    let state = match entry.state {
        State::Applied => "",
        State::Sending => " [sending]",
        State::Unknown => " [no answer]",
    };
    let origin = match (entry.run == app.safety.run, entry.kept) {
        (true, _) => "",
        (false, true) => " [kept, earlier run]",
        (false, false) => " [earlier run]",
    };
    Line::from(vec![
        Span::raw(entry.label()),
        Span::raw(format!(": {} (undo: {undo})", entry.written.text)),
        Span::styled(format!("{state}{origin}"), theme.warn()),
    ])
}

fn ledger(app: &App, notice: Option<&str>, confirm: bool, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(notice) = notice {
        lines.push(Line::styled(notice.to_string(), theme.warn()));
    }
    if app.safety.ledger.is_empty() {
        lines.push(Line::raw("No changes in effect."));
    }
    for entry in &app.safety.ledger {
        lines.push(entry_line(app, entry, theme));
    }
    lines.push(Line::raw(""));
    if confirm {
        lines.push(Line::styled(
            format!(
                "Revert {} change(s), newest first? y yes | Esc no",
                app.safety.ledger.len()
            ),
            theme.warn(),
        ));
    } else {
        lines.push(Line::styled(
            "R revert all (relinquish our slots, restore old values) | Esc close",
            theme.dim(),
        ));
    }
    lines
}

fn exit_guard(app: &App, error: Option<&str>, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw(format!(
        "{} change(s) made through this tool are still in effect:",
        app.safety.ledger.len()
    ))];
    if app.safety.pending.is_some() {
        lines.push(Line::styled(
            "A request is still out; its outcome is recorded either way.",
            theme.warn(),
        ));
    }
    for entry in &app.safety.ledger {
        lines.push(entry_line(app, entry, theme));
    }
    lines.push(Line::raw(""));
    if let Some(error) = error {
        lines.push(Line::styled(error.to_string(), theme.danger()));
    }
    lines.push(Line::styled(
        "r revert all, then quit | k keep them, log it, quit | Esc cancel",
        theme.accent(),
    ));
    lines
}
