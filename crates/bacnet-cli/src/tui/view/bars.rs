//! The status bar and the footer (#1894). Both fit their width by
//! priority ([`super::fit`]): at 80 columns the least useful parts give way
//! first, and `? help` and `q quit` always stay. A footer notice sits
//! beside the key hints rather than over them; from [`WIDE_WIDTH`] columns
//! the hints give way so it shows in full where they can (#1927).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::fit::{self, ALWAYS, Part, hint, span};
use super::{Theme, WIDE_WIDTH, ellipsize, text_width};
use crate::tui::app::command_bar;
use crate::tui::app::watch::Screen;
use crate::tui::app::{App, Link, Overlay};
use crate::tui::safety::Level as Safety;

/// Columns the status bar keeps for `? help` at its right.
const HELP_WIDTH: u16 = 9;

/// Hints kept at this priority or above stay clear of a footer notice:
/// the way back (or a failed link's recovery key), `? help` and `q quit`.
/// From [`WIDE_WIDTH`], when giving up the way back shows the whole
/// notice, only `? help` and `q quit` do; never while the link has failed.
const RESERVED: u8 = 8;

/// `bacnet | BIP 10.0.1.5:47808 | up | READ-ONLY | COV 0 FD held | drop 0`
/// and `? help`. `COV` and `FD` are the footprint: subscriptions held on
/// devices, and the foreign-device registration with its state (shown only
/// with one). An armed write session turns the bar red with its countdown;
/// `ledger n` counts changes still in effect.
pub(super) fn status_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let level = app.level();
    let bar = match level {
        Safety::Armed { .. } => theme.danger(),
        _ => theme.bar(),
    };
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(HELP_WIDTH)]).areas(area);
    let room = usize::from(left_area.width).saturating_sub(1);
    let mut spans = vec![Span::styled(" ", bar)];
    spans.extend(fit::fit(
        status_parts(app, theme, bar),
        &Span::styled(" | ", bar),
        room,
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), left_area);
    frame.render_widget(
        Paragraph::new("? help ")
            .style(bar)
            .alignment(Alignment::Right),
        right_area,
    );
}

/// The status bar's fields, in order, each with what it gives way at.
fn status_parts(app: &App, theme: &Theme, bar: Style) -> Vec<Part> {
    let transport = if app.transport.is_empty() {
        "no profile"
    } else {
        app.transport
    };
    let state = match &app.link {
        Link::Down => "disconnected",
        Link::Disconnecting => "disconnecting",
        Link::Picking if app.transport == "MSTP" => "confirm joining",
        Link::Picking => "choose an interface",
        Link::Connecting => "connecting",
        Link::Up { .. } => "up",
        Link::Failed(_) => "failed",
        Link::Passive if app.monitor.is_some() => "listen only",
        Link::Passive if app.passive_listener.is_some() => "listening",
        Link::Passive => "no client",
    };
    let level = match app.level() {
        Safety::Passive => Span::styled("PASSIVE", theme.safe()),
        Safety::ReadOnly | Safety::Expired => Span::styled("READ-ONLY", theme.safe()),
        Safety::Armed { dangerous } => {
            let left = app.safety.armed.as_ref().map_or(0, |armed| {
                armed.until.saturating_duration_since(app.now).as_secs()
            });
            let name = if dangerous { "WRITE+DANGER" } else { "WRITE" };
            Span::styled(format!("{name} {}:{:02}", left / 60, left % 60), bar)
        }
    };
    let mut parts = vec![
        Part::new(1, vec![span("bacnet", bar)]),
        strip(app, theme, bar),
    ];
    // The local address goes before the fields after it do; on SC before
    // the hub state, as the SC screen shows the VMAC anyway.
    let address = if app.transport == "SC" { 3 } else { 4 };
    parts.push(match &app.link {
        Link::Up { local } => Part::new(address, vec![span(format!("{transport} {local}"), bar)])
            .or(8, vec![span(transport, bar)]),
        Link::Passive if app.passive_listener.is_some() => Part::new(
            address,
            vec![span(
                format!("{transport} {}", app.passive_listener.as_deref().unwrap()),
                bar,
            )],
        )
        .or(8, vec![span(transport, bar)]),
        _ => Part::new(8, vec![span(transport, bar)]),
    });
    parts.push(Part::new(ALWAYS, vec![span(state, bar)]));
    if let Some(hub) = hub_state(app) {
        parts.push(Part::new(4, vec![span(hub, bar)]));
    }
    parts.push(Part::new(ALWAYS, vec![level]));
    // Subscriptions held on devices matter more than none.
    let subscriptions = app.watch.stats.subscriptions;
    parts.push(Part::new(
        if subscriptions > 0 { 5 } else { 2 },
        vec![span(format!("COV {subscriptions}"), bar)],
    ));
    if let Some(registration) = app.network.registration_state() {
        parts.push(Part::new(
            5,
            vec![span(format!("FD {}", registration.label()), bar)],
        ));
    }
    if !app.safety.ledger.is_empty() {
        parts.push(Part::new(
            7,
            vec![span(format!("ledger {}", app.safety.ledger.len()), bar)],
        ));
    }
    parts.push(Part::new(
        6,
        vec![span(format!("drop {}", app.dropped), bar)],
    ));
    parts
}

/// The numbered screen strip (#1895), with only the screens that apply to
/// the session and the one shown in brackets: `1 Devices 2 Browse [3 Watch]
/// 4 Network 7 Capture`, or in less room `1 2 [3 Watch] 4 7`. It sits in
/// the status bar so it costs no row at 80x24.
fn strip(app: &App, theme: &Theme, bar: Style) -> Part {
    let tabs = crate::tui::help::screens(app);
    let form = |labels: bool| {
        let mut spans = Vec::new();
        for (index, tab) in tabs.iter().enumerate() {
            if index > 0 {
                spans.push(span(" ", bar));
            }
            if tab.current {
                spans.push(span(
                    format!("[{} {}]", tab.key, tab.label),
                    theme.selected(),
                ));
            } else if labels {
                spans.push(span(format!("{} {}", tab.key, tab.label), bar));
            } else {
                spans.push(span(tab.key.to_string(), bar));
            }
        }
        spans
    };
    Part::new(2, form(true)).or(7, form(false))
}

/// The SC hub connector in a few words, once the SC worker has reported:
/// `hub primary`, `hub failover`, `hub connecting`, `hub down`.
fn hub_state(app: &App) -> Option<String> {
    use bacnet_transport::sc::ScConnectionState;
    let (link, _) = app.sc.link.as_ref()?;
    Some(match (link.state, link.hub) {
        (ScConnectionState::Connected, Some(hub)) => {
            format!("hub {}", crate::tui::sc::hub_name(hub))
        }
        (ScConnectionState::Connecting, _) => "hub connecting".into(),
        _ => "hub down".into(),
    })
}

/// Key hints, the filter line, or the command bar. A notice (a flash, or
/// a failed connection's summary) takes the left and the hints sit at the
/// right, cut to the ones that fit. `tab_focus`: the Network screen's wide
/// layout, where Tab moves the focus between panes.
pub(super) fn footer(frame: &mut Frame, area: Rect, app: &App, theme: &Theme, tab_focus: bool) {
    let width = usize::from(area.width);
    if let Overlay::Command(text) = &app.overlay {
        frame.render_widget(Paragraph::new(command_line(text, width, theme)), area);
        return;
    }
    if app.filter_editing {
        let line = Line::from(vec![
            Span::styled(" /", theme.accent()),
            Span::styled(format!("{}_", app.filter_text()), theme.selected()),
            Span::styled("  Enter keep  Esc clear", theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let flash = app.flash.as_ref().map(|f| (f.text.as_str(), theme.warn()));
    if app.overlay != Overlay::None {
        // The open dialog lists its own keys; a notice set while it is
        // open still shows.
        if let Some((text, style)) = flash {
            let line = Line::styled(
                format!(" {}", ellipsize(text, width.saturating_sub(1))),
                style,
            );
            frame.render_widget(Paragraph::new(line), area);
        }
        return;
    }
    let failed;
    let notice = match (&app.link, flash) {
        (_, Some(flash)) => Some(flash),
        (Link::Failed(failure), None) => {
            failed = format!("Connection failed: {}", failure.summary);
            Some((failed.as_str(), theme.danger()))
        }
        _ => None,
    };
    let parts = screen_hints(app, theme, tab_focus);
    let Some((text, style)) = notice else {
        let line = fit::hints(parts, width.saturating_sub(1));
        let mut spans = vec![Span::raw(" ")];
        spans.extend(line.spans);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    };
    // The way back (or a failed link's recovery key), `? help` and `q quit`
    // are kept clear of the notice; the other hints get what the notice
    // leaves, the lowest priority giving way first. On a wide terminal the
    // way back gives way too, but only when that shows the whole notice;
    // the recovery key never does.
    let room = |keep| width.saturating_sub(1 + fit::reserve(&parts, keep) + 2);
    let failed = matches!(app.link, Link::Failed(_));
    let whole = text_width(text) <= room(ALWAYS);
    let reserved = if area.width >= WIDE_WIDTH && whole && !failed {
        ALWAYS
    } else {
        RESERVED
    };
    let notice_room = room(reserved);
    let notice = ellipsize(text, notice_room);
    let used = 1 + text_width(&notice);
    let hints = fit::hints(parts, width.saturating_sub(used + 2));
    let [left, right] =
        Layout::horizontal([Constraint::Length(used as u16), Constraint::Min(0)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled(format!(" {notice}"), style)),
        left,
    );
    frame.render_widget(Paragraph::new(hints).alignment(Alignment::Right), right);
}

/// The command bar: what is typed, then the syntax of the commands it
/// matches (#1895), from the table the parser reads, cut to fit before
/// the keys.
fn command_line(text: &str, width: usize, theme: &Theme) -> Line<'static> {
    const KEYS: &str = "  Enter run  Esc close";
    let typed = format!("{text}_");
    let matches = command_bar::matching(text);
    let syntax = if matches.is_empty() {
        "no such command".to_string()
    } else {
        let forms: Vec<&str> = matches
            .iter()
            .flat_map(|s| s.usage.iter().copied())
            .collect();
        forms.join(" | ")
    };
    let room = width.saturating_sub(2 + text_width(&typed) + 2 + text_width(KEYS));
    let mut spans = vec![
        Span::styled(" :", theme.accent()),
        Span::styled(typed, theme.selected()),
    ];
    if room > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(ellipsize(&syntax, room), theme.warn()));
    }
    spans.push(Span::styled(KEYS, theme.dim()));
    Line::from(spans)
}

/// The footer's key hints for the screen shown, each with its priority:
/// `? help` and `q quit` always stay, then the way back, then the
/// screen's main keys. Only keys that work there are listed.
fn screen_hints(app: &App, theme: &Theme, tab_focus: bool) -> Vec<Part> {
    let profiles = app.connection.is_some();
    let h = |keep, key, what| hint(theme, keep, key, what);
    let mut parts = Vec::new();
    match app.screen {
        Screen::Network if app.networks_tab() => {
            if !app.safety.passive {
                parts.push(h(6, "r", "ask routers"));
            }
            parts.push(h(5, "Tab", "tables"));
            parts.push(h(3, ":find", "NAME"));
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Network => {
            parts.push(h(6, "r", "read tables"));
            parts.push(h(4, "m", "survey BBMDs"));
            parts.push(if tab_focus {
                h(4, "Tab", "focus pane/networks").or_hint(theme, 6, "Tab", "focus pane")
            } else {
                h(4, "Tab", "BDT/FDT/matrix/networks").or_hint(theme, 6, "Tab", "tables")
            });
            // After `:bbmd OTHER`, the way back to our own BBMD's tables.
            parts.push(match app.away_from_own_bbmd() {
                Some(_) => h(7, ":bbmd", "own BBMD"),
                None => h(3, ":bbmd", "ADDRESS"),
            });
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Sc if app.transport == "SC" => {
            parts.push(
                h(4, "Tab", "certificates/doctor/credentials").or_hint(theme, 6, "Tab", "view"),
            );
            parts.push(h(4, "p", "profiles"));
            parts.push(h(5, "x", "disconnect"));
            parts.push(h(8, "Esc", "back"));
        }
        // Off SC only leaving, or connecting a profile, does anything.
        Screen::Sc => {
            if profiles {
                parts.push(h(6, "p", "profiles"));
            }
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Mstp if app.mstp.station.is_some() => {
            parts.push(h(4, "z", "clear counters"));
            parts.push(h(5, "Space", "pause/follow"));
            parts.push(h(6, "x", "disconnect"));
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Mstp if app.monitor.is_some() => {
            parts.push(h(6, "Tab", "view"));
            if app
                .monitor
                .as_ref()
                .is_some_and(|m| m.live && m.scan.as_ref().is_none_or(|scan| !scan.active()))
            {
                parts.push(h(7, "J", "safe join"));
            }
            parts.push(h(2, ":", "scan commands"));
            parts.push(h(3, "z", "clear"));
            parts.push(h(4, "Space", "pause"));
            parts.push(h(5, "x", "stop"));
            parts.push(h(8, "Esc", "back"));
        }
        // Before a station joins: joining again, or a profile.
        Screen::Mstp => {
            // With profiles on, `c` opens the profile dialog like `p`.
            if profiles {
                parts.push(h(6, "p", "profiles"));
            } else if app.transport == "MSTP" && matches!(app.link, Link::Down | Link::Failed(_)) {
                parts.push(h(6, "c", "join"));
            }
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Capture => {
            parts.push(h(5, "Space", "pause"));
            parts.push(h(4, "Tab", "pane"));
            parts.push(h(6, "/", "filter"));
            parts.push(h(3, "e", "export"));
            parts.push(h(2, "z", "clear"));
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Watch => {
            parts.push(h(6, "x", "remove"));
            parts.push(h(5, ":watch", "DEV OBJ"));
            parts.push(h(8, "Esc", "back"));
        }
        Screen::Main | Screen::Browse => main_hints(app, theme, &mut parts),
    }
    parts.push(h(ALWAYS, "?", "help"));
    parts.push(h(ALWAYS, "q", "quit"));
    parts
}

/// The device table's and the browser's hints, by the link's state.
fn main_hints(app: &App, theme: &Theme, parts: &mut Vec<Part>) {
    let profiles = app.connection.is_some();
    let h = |keep, key, what| hint(theme, keep, key, what);
    match &app.link {
        Link::Up { .. } if app.screen == Screen::Browse => {
            parts.push(h(7, "Enter", "read"));
            parts.push(h(6, "a", "watch"));
            parts.push(h(5, "Tab", "pane"));
            parts.push(h(4, "/", "filter"));
            parts.push(h(3, "r", "refresh"));
            parts.push(h(8, "Esc", "devices"));
        }
        Link::Up { .. } => {
            parts.push(h(7, "Enter", "browse"));
            parts.push(h(7, "d", "Who-Is"));
            parts.push(h(5, "/", "filter"));
            parts.push(h(4, "s", "sort"));
            parts.push(h(3, "x", "disconnect"));
        }
        // The summary is beside these; the whole error is on the device
        // screen and in the log.
        // With profiles on, `c` only opens the profile dialog, as `p`
        // does, so one hint names it; it stays clear of the summary.
        Link::Failed(_) => {
            parts.push(match profiles {
                true => h(8, "p", "settings"),
                false => h(8, "c", "retry"),
            });
            parts.push(h(4, "L", "log"));
        }
        link => {
            if profiles {
                parts.push(h(6, "p", "profiles"));
            } else if matches!(link, Link::Down) {
                parts.push(h(7, "c", "connect"));
            }
            if matches!(link, Link::Connecting | Link::Picking) {
                parts.push(h(5, "x", "cancel"));
            }
            parts.push(h(4, "L", "log"));
        }
    }
}
