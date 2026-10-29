//! Profile editor and resolved connection preview at the supported terminal sizes.

use super::{Theme, centered};
use crate::tui::app::connection::{Connection, Screen};
use crate::tui::profile::{Profile, Transport, Writes};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Wrap},
};

/// `live`: a session is connecting or up, so the dialog offers disconnect.
pub(super) fn draw(
    frame: &mut Frame,
    area: Rect,
    connection: &Connection,
    live: bool,
    theme: &Theme,
) {
    let mut lines = match connection.screen {
        Screen::List => list(connection, theme),
        Screen::Edit => editor(connection, theme),
        Screen::Preview => preview(connection, live, theme),
    };
    if connection.pending {
        lines.push(Line::styled("Working…", theme.warn()));
    }
    if let Some(error) = &connection.error {
        lines.push(Line::styled(error.clone(), theme.danger()));
    }
    // Grow past the usual 22 rows when a taller terminal has the room,
    // counting each line by the rows it wraps to (borders and padding
    // take four columns).
    let width = area.width.saturating_sub(4).min(96);
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let rows: usize = lines
        .iter()
        .map(|line| line.width().div_ceil(inner).max(1))
        .sum();
    let height = u16::try_from(rows + 2).unwrap_or(u16::MAX).max(22);
    let box_area = centered(area, width, height);
    frame.render_widget(Clear, box_area);
    let block = Block::bordered()
        .title(Line::styled(" Connection profiles ", theme.accent()))
        .title_bottom(Line::styled(
            if connection.screen == Screen::Edit {
                " Esc discard / back | READ-ONLY "
            } else {
                " Esc close | READ-ONLY "
            },
            theme.dim(),
        ))
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        box_area,
    );
}

fn list<'a>(connection: &'a Connection, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::raw("Choose a profile to review before connecting."),
        Line::raw(""),
    ];
    if connection.names.is_empty() {
        lines.push(Line::raw(
            "No profiles. Press n for a B/IP profile or s for BACnet/SC.",
        ));
    }
    let offset = connection.selected.saturating_sub(8);
    for (i, name) in connection.names.iter().enumerate().skip(offset).take(9) {
        let selected = i == connection.selected;
        lines.push(Line::styled(
            format!("{} {name}", if selected { ">" } else { " " }),
            if selected {
                theme.selected()
            } else {
                theme.dim()
            },
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw(
        "Enter preview | Up/Down choose | n/s/m new B/IP, SC, MS/TP | r reload",
    ));
    lines.push(Line::styled(
        format!("Config: {}", connection.root),
        theme.dim(),
    ));
    lines
}

fn editor<'a>(connection: &'a Connection, theme: &Theme) -> Vec<Line<'a>> {
    let kind = match connection.transport {
        Transport::Bip => "B/IP",
        Transport::Sc => "BACnet/SC",
        Transport::Mstp => "MS/TP",
    };
    let mut lines = vec![Line::raw(format!(
        "{} {kind} profile; Enter saves it without connecting.",
        if connection.creating {
            "Create a"
        } else {
            "Edit the"
        }
    ))];
    for (i, label) in connection.labels().iter().enumerate() {
        let focused = i == connection.focus;
        let field = &connection.fields[i];
        let tail: String = field
            .chars()
            .rev()
            .take(42)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let locked = connection.identity_locked(i);
        let text = format!(
            "{} {label:<13} {tail}{}",
            if focused { ">" } else { " " },
            match (locked, focused) {
                (true, _) => " (read-only)",
                (false, true) => "_",
                (false, false) => "",
            }
        );
        lines.push(Line::styled(
            text,
            if focused {
                theme.selected()
            } else {
                theme.dim()
            },
        ));
    }
    lines.push(Line::raw(
        "Tab/Shift-Tab fields | Backspace erase | Enter save",
    ));
    match connection.transport {
        Transport::Bip => {
            lines.push(Line::raw(
                "Blank BBMD = Normal; otherwise Foreign Device (IP[:port]).",
            ));
            lines.push(Line::raw(
                "Blank renewal = half TTL; interface 0.0.0.0 = all interfaces.",
            ));
            lines.push(Line::raw(
                "Group: a multicast group (B/IP-M) on an explicit interface.",
            ));
        }
        Transport::Sc => {
            lines.push(Line::raw(
                "Hub URLs are wss://. Files are PEM paths; only the paths are saved.",
            ));
            lines.push(Line::raw(
                "Reconnect on/off: redial a lost hub; a failover hub needs it on.",
            ));
            lines.push(Line::raw(
                if connection.identity_locked(crate::tui::app::connection::UUID_FIELD) {
                    "VMAC / UUID are kept for good; to change them, edit the profile file."
                } else {
                    "Blank VMAC / UUID: generated once on save, then kept."
                },
            ));
        }
        Transport::Mstp => {
            lines.push(Line::raw(
                "Port as bacnet mstp-ports lists it. MAC: unused on the trunk.",
            ));
            lines.push(Line::raw(
                "Connecting asks you to confirm joining the token ring (y).",
            ));
        }
    }
    lines.push(Line::raw(
        "Writes: never or session (W arms). Dangerous: allows priority 6.",
    ));
    lines
}

fn sc_lines(profile: &Profile, lines: &mut Vec<Line<'_>>) {
    let Some(s) = &profile.sc else { return };
    lines.push(Line::raw(format!("Hub: {}", s.hub_url)));
    if let Some(failover) = &s.failover_hub_url {
        lines.push(Line::raw(format!("Failover hub: {failover}")));
    }
    lines.push(Line::raw(if s.reconnect {
        format!(
            "Reconnect: on, {} ms doubling to {} ms, {} attempts",
            s.reconnect_initial_delay_ms, s.reconnect_max_delay_ms, s.reconnect_max_retries,
        )
    } else {
        "Reconnect: off (a lost hub stays down)".into()
    }));
    lines.push(Line::raw(format!("CA file:   {}", s.ca.display())));
    lines.push(Line::raw(format!("Cert file: {}", s.cert.display())));
    lines.push(Line::raw(format!("Key file:  {}", s.key.display())));
    let unset = || "generated on connect".to_owned();
    lines.push(Line::raw(format!(
        "VMAC: {}   Device UUID: {}",
        s.vmac.map_or_else(unset, |v| v.to_string()),
        s.device_uuid.map_or_else(unset, |v| v.to_string()),
    )));
    lines.push(Line::raw(format!(
        "APDU timeout: {} ms   Pacing: {} ms   Connect timeout: {} ms",
        s.timeout_ms, s.min_interval_ms, s.connect_timeout_ms
    )));
}

fn mstp_lines(profile: &Profile, lines: &mut Vec<Line<'_>>) {
    let Some(m) = &profile.mstp else { return };
    lines.push(Line::raw(format!(
        "Port: {}   Station (MAC): {}   Baud: {}",
        m.port, m.mac, m.baud
    )));
    lines.push(Line::raw(format!(
        "Max master: {}   Info frames: {}",
        m.max_master, m.max_info_frames
    )));
    lines.push(Line::raw(format!(
        "APDU timeout: {} ms   Pacing: {} ms",
        m.timeout_ms, m.min_interval_ms
    )));
}

fn bip_lines<'a>(profile: &Profile, lines: &mut Vec<Line<'a>>, theme: &Theme) {
    let Some(b) = &profile.bip else { return };
    lines.push(Line::raw(format!(
        "Mode: {}",
        if b.bbmd.is_some() {
            "Foreign Device"
        } else if b.multicast.is_some() {
            "Normal, multicast (B/IP-M)"
        } else {
            "Normal"
        }
    )));
    match b.multicast {
        // A member broadcasts to its group, not the subnet (Annex J.8).
        Some(group) => lines.push(Line::raw(format!(
            "Bind: {}:{}   Group: {}:{}   TTL: {}",
            b.interface,
            b.port,
            group,
            b.multicast_port.unwrap_or(b.port),
            b.multicast_ttl
        ))),
        None => lines.push(Line::raw(format!(
            "Bind: {}:{}   Broadcast: {}",
            b.interface, b.port, b.broadcast
        ))),
    }
    lines.push(Line::raw(format!(
        "APDU timeout: {} ms   Pacing: {} ms",
        b.timeout_ms, b.min_interval_ms
    )));
    if let Some(bbmd) = b.bbmd {
        lines.push(Line::raw(format!("BBMD: {bbmd}   TTL: {} s", b.ttl)));
        lines.push(Line::raw(format!(
            "Renewal: {}",
            b.renewal_secs
                .map_or_else(|| "half TTL".into(), |n| format!("{n} s"))
        )));
        lines.push(Line::styled(
            "Local connection does not prove BBMD acceptance; see L log.",
            theme.dim(),
        ));
    }
}

fn preview<'a>(connection: &'a Connection, live: bool, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![];
    match connection.resolved() {
        Err(error) => {
            lines.push(Line::styled(
                format!("{} — resolved connection", connection.name),
                theme.accent(),
            ));
            lines.push(Line::styled(error, theme.danger()));
        }
        Ok(profile) => {
            let kind = match profile.transport {
                Transport::Bip => "B/IP",
                Transport::Sc => "BACnet/SC",
                Transport::Mstp => "MS/TP",
            };
            lines.push(Line::styled(
                format!("{} — resolved {kind} connection", connection.name),
                theme.accent(),
            ));
            bip_lines(&profile, &mut lines, theme);
            sc_lines(&profile, &mut lines);
            mstp_lines(&profile, &mut lines);
            lines.push(Line::raw(format!(
                "Writes: {}; dangerous: {}. Read-only until W arms a session.",
                match profile.safety.writes {
                    Writes::Never => "never",
                    Writes::Session => "session",
                },
                profile.safety.dangerous
            )));
            if !connection.overrides.is_empty() {
                lines.push(Line::styled(
                    "Explicit CLI overrides are included above; saved values stay separate.",
                    theme.warn(),
                ));
            }
            for error in &connection.checks.errors {
                lines.push(Line::styled(format!("✗ {error}"), theme.danger()));
            }
            for warning in &connection.checks.warnings {
                lines.push(Line::styled(format!("! {warning}"), theme.warn()));
            }
            if !connection.checks.errors.is_empty() {
                lines.push(Line::styled(
                    "Connecting is refused until these are fixed (r re-checks).",
                    theme.danger(),
                ));
            } else if !connection.checks.warnings.is_empty() {
                lines.push(Line::styled(
                    if connection.accepted == connection.checks.warnings {
                        "Warnings accepted for this connection."
                    } else {
                        "y accept these warnings for this connection | fix them and r re-check"
                    },
                    theme.warn(),
                ));
            }
            lines.push(Line::raw(""));
            if live {
                lines.push(Line::styled("x disconnect", theme.accent()));
            } else {
                match profile.authorized_at_unix {
                    Some(at) => {
                        lines.push(Line::raw(format!("Authorization saved at Unix time {at}.")));
                        lines.push(Line::styled("Enter / c connect", theme.accent()));
                    }
                    None => {
                        lines.push(Line::styled(
                            "First use requires your explicit authorization:",
                            theme.warn(),
                        ));
                        lines.push(Line::raw("I am authorized to use BACnet on this network."));
                        lines.push(Line::styled(
                            "a attest, save authorization and connect | Esc cancel",
                            theme.warn(),
                        ));
                    }
                }
            }
        }
    }
    lines.push(Line::raw(
        "e edit | r reload from disk | p profiles | n/s/m new B/IP, SC, MS/TP",
    ));
    lines
}
