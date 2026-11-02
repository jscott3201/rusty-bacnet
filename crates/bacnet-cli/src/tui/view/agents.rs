//! The agent pane (screen 8, #2185): what the attached agents asked for
//! and what it cost, newest first, under the requests waiting here for the
//! person to answer, each with its age.
//!
//! The ages matter because an agent's host often gives up on a tool call
//! after about 60 s, while the dialog waits up to 120 s: a request older
//! than that may answer an agent that no longer listens. Every agent
//! string (its client's claimed name, the target) was cleaned by the
//! listener and is cleaned again here, as device text is.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_mcp::attach::{CallOutcome, CallRow, MAX_QUEUED};
use bacnet_ops::limits::METER_WINDOW_SECS;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Row, Table};

use super::{FULL_WIDTH, Theme, ellipsize, sanitize, text_width};
use crate::tui::app::App;
use crate::tui::app::agents::Held;
use crate::tui::app::safety::Modal;
use crate::tui::message::McpListener;

/// How long a host commonly lets a tool call run: past it, the pane marks
/// a waiting request as one its agent may have stopped waiting for.
const HOST_TIMEOUT: Duration = Duration::from_secs(60);

/// `HH:MM:SS`, UTC, as the capture list shows times.
fn clock(time: SystemTime) -> String {
    let day = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3_600, day % 3_600 / 60, day % 60)
}

/// The agent as a cell: its number, then the name its client claims.
fn who(agent: u64, client: Option<&str>) -> String {
    match client {
        Some(name) => format!("{agent} {}", sanitize(name)),
        None => agent.to_string(),
    }
}

/// How a call ended, in a word or two.
fn outcome(outcome: &CallOutcome) -> String {
    match outcome {
        CallOutcome::Ok => "ok".into(),
        CallOutcome::Refused(why) => format!("refused: {}", sanitize(why)),
        CallOutcome::Declined(why) => sanitize(why),
        CallOutcome::Stale => "stale".into(),
        CallOutcome::Error(what) => format!("error: {}", sanitize(what)),
    }
}

/// What a call cost: the requests charged to it and, when there is room,
/// how long it took.
fn cost(row: &CallRow, wide: bool) -> String {
    match wide {
        true => format!("{} req {:.1} s", row.requests, row.elapsed.as_secs_f64()),
        false => format!("{} req", row.requests),
    }
}

/// The heading: agents attached, their share of the request meter, the
/// arming here, and the calls the ring dropped; shorter when not `wide`.
fn title(app: &App, wide: bool) -> String {
    let attached = app.mcp.as_ref().and_then(McpListener::agents);
    let attached = attached.map_or("listener off".to_string(), |n| format!("{n} attached"));
    let per_s = |n: usize| n as f64 / METER_WINDOW_SECS as f64;
    let stats = &app.watch.stats;
    let (agents, all) = (per_s(stats.agent_requests), per_s(stats.recent_requests));
    let armed = match app.safety.armed {
        Some(_) => "armed",
        None => "not armed",
    };
    let dropped = match (app.safety.agents.pane.calls.dropped, wide) {
        (0, _) => String::new(),
        (n, true) => format!(" | {n} older dropped"),
        (n, false) => format!(" | {n} dropped"),
    };
    match wide {
        true => format!(
            " Agents: {attached} | requests {agents:.1}/s of {all:.1}/s | writes here {armed}{dropped} "
        ),
        false => format!(" Agents: {attached} | {agents:.1}/s of {all:.1}/s | {armed}{dropped} "),
    }
}

/// One waiting request as a line `width` wide, shorter when not `wide`:
/// what it would do, its age, its countdown and the host warning first,
/// then who asks, the agent's claimed name cut to the room left, so no
/// name can push the countdown or the warning off screen.
fn waiting_line(held: &Held, theme: &Theme, wide: bool, width: usize) -> Line<'static> {
    let (what, device, age) = (held.what, held.device, held.age.as_secs());
    let mut text = match wide {
        true => format!("  {what} device {device}, waiting {age} s"),
        false => format!("  {what} dev {device}: {age} s"),
    };
    if let Some(left) = held.left {
        text.push_str(&match wide {
            true => format!(", on screen, declined in {} s", left.as_secs()),
            false => format!(", shown, {} s left", left.as_secs()),
        });
    }
    let late = held.age >= HOST_TIMEOUT;
    if late {
        text.push_str(match wide {
            true => " (its host may have stopped waiting)",
            false => " (host may be gone)",
        });
    }
    let who = match &held.client {
        Some(name) => format!(", agent {} \"{}\"", held.agent, sanitize(name)),
        None => format!(", agent {}", held.agent),
    };
    let room = width.saturating_sub(text_width(&text));
    text.push_str(&ellipsize(&who, room));
    match late {
        true => Line::styled(text, theme.warn()),
        false => Line::raw(text),
    }
}

/// The waiting requests: a heading, one line each, and the host timeout
/// note; shorter when not `wide`.
fn waiting(app: &App, theme: &Theme, wide: bool, width: usize) -> Vec<Line<'static>> {
    let opened = match &app.overlay {
        crate::tui::app::Overlay::Safety(modal) => match **modal {
            Modal::Confirm { req, opened, .. } if app.safety.agents.shows(req) => Some(opened),
            _ => None,
        },
        _ => None,
    };
    let held = app.safety.agents.held_at(app.now, opened);
    if held.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::styled(
        format!(
            "Waiting for you here ({} of at most {MAX_QUEUED}):",
            held.len()
        ),
        theme.accent(),
    )];
    lines.extend(held.iter().map(|h| waiting_line(h, theme, wide, width)));
    let note = match wide {
        true => {
            "Hosts often end a tool call after about 60 s; raise the host's tool timeout if it has one."
        }
        false => "Hosts often give up after about 60 s: raise their tool timeout.",
    };
    lines.push(Line::styled(note, theme.dim()));
    lines.push(Line::raw(""));
    lines
}

/// Draw the pane in `area`.
pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let wide = area.width >= FULL_WIDTH;
    let block = Block::bordered()
        .title(Line::styled(title(app, wide), theme.accent()))
        .border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let waiting = waiting(app, theme, wide, usize::from(inner.width));
    let [top, calls] = Layout::vertical([
        Constraint::Length(u16::try_from(waiting.len()).unwrap_or(u16::MAX)),
        Constraint::Min(2),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(waiting), top);
    let rows = &app.safety.agents.pane.calls.rows;
    if rows.is_empty() {
        let text = "No agent calls yet. Agents attach with `bacnet mcp connect --profile NAME`.";
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), calls);
        return;
    }
    let (agent, tool, result, spend) = match wide {
        true => (20, 22, 26, 13),
        false => (10, 14, 18, 7),
    };
    let fixed = 8 + agent + tool + result + spend + 5;
    let target = usize::from(calls.width).saturating_sub(fixed).max(8);
    // The newest first; scrolled, clamped so the last page stays full.
    let height = usize::from(calls.height.saturating_sub(1)).max(1);
    let pane = &app.safety.agents.pane;
    let first = pane.scroll.get().min(rows.len().saturating_sub(height));
    pane.scroll.set(first);
    let lines: Vec<Row> = rows
        .iter()
        .skip(first)
        .take(height)
        .map(|row| {
            let name = match wide {
                true => row.tool.clone(),
                false => row.tool.trim_start_matches("bacnet_").to_string(),
            };
            let line = Row::new([
                clock(row.at),
                ellipsize(&who(row.agent, row.client.as_deref()), agent),
                ellipsize(&sanitize(&name), tool),
                ellipsize(&sanitize(&row.target), target),
                ellipsize(&outcome(&row.outcome), result),
                cost(row, wide),
            ]);
            match row.outcome {
                CallOutcome::Ok => line,
                _ => line.style(theme.warn()),
            }
        })
        .collect();
    let length = |n: usize| Constraint::Length(u16::try_from(n).unwrap_or(u16::MAX));
    let widths = [
        length(8),
        length(agent),
        length(tool),
        length(target),
        length(result),
        length(spend),
    ];
    let header =
        Row::new(["Time", "Agent", "Tool", "Target", "Outcome", "Cost"]).style(theme.accent());
    frame.render_widget(
        Table::new(lines, widths).header(header).column_spacing(1),
        calls,
    );
}
