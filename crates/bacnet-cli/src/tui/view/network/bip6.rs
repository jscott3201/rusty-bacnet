//! A B/IPv6 foreign device on the Network screen's panel (#2378). Its BBMD,
//! status and counts lines are B/IP's; this draws its status words and its
//! last three lines. The status says how the BBMD answered the latest
//! registration and when (#2388); the lines count what was sent where B/IP
//! shows the last BVLC-Result, a countdown and our FDT entry, since B/IPv6
//! reads no tables.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use bacnet_transport::bip6::Bip6ForeignStatus;

use super::{NEXT_ONCE_RESOLVED, Theme, WAITING, push_lookups};
use crate::tui::app::App;
use crate::tui::app::network::Registration;

/// The status words and their style, and the counters: broadcasts
/// refused while waiting (when any were), and the host name's lookups with
/// why the newest failed. The words say how the BBMD answered the latest
/// registration, and how long ago (#2388).
pub(super) fn status(
    app: &App,
    status: &Bip6ForeignStatus,
    theme: &Theme,
) -> (String, Style, String) {
    let ago = app
        .network
        .bip6_answer_age(app.now)
        .map_or(0, |age| age.as_secs());
    let (word, style) = match app.network.registration_state() {
        Some(Registration::Waiting) => (WAITING.to_string(), theme.warn()),
        Some(Registration::Held) => (format!("registered, accepted {ago} s ago"), theme.accent()),
        Some(Registration::Refused) => (format!("REFUSED {ago} s ago"), theme.danger()),
        Some(Registration::NoAnswer { unanswered: 1 }) => (
            "no answer to the last registration".to_string(),
            theme.warn(),
        ),
        Some(Registration::NoAnswer { unanswered }) => (
            format!("no answer to the last {unanswered} registrations"),
            theme.warn(),
        ),
        _ if status.registrations_sent > 0 => (
            "registration sent, waiting for the BBMD's answer".to_string(),
            theme.dim(),
        ),
        _ => ("first registration not sent yet".to_string(), theme.dim()),
    };
    let mut counts = String::new();
    if status.broadcasts_refused > 0 {
        counts = format!("broadcasts refused {}", status.broadcasts_refused);
    }
    let why = status
        .lookups
        .last_failure
        .map(crate::foreign::bip6_lookup_failure);
    push_lookups(
        &mut counts,
        crate::foreign::bip6_lookups(&status.lookups),
        why,
    );
    (word, style, counts.trim_start().to_string())
}

/// The last three lines: the registrations sent and refused, when the
/// next goes, and the releases of the BBMDs it left.
pub(super) fn rows(frame: &mut Frame, areas: [Rect; 3], status: &Bip6ForeignStatus, theme: &Theme) {
    // The totals survive a restart and count every BBMD, not only this
    // one, so they say so rather than sit beside the BBMD's status.
    let sent = format!(
        "Registrations sent {}, refused {}, to every BBMD this device has used",
        status.registrations_sent, status.registrations_refused
    );
    // As the state reads it: no address yet is waiting.
    let next = if status.bbmd.is_none() {
        NEXT_ONCE_RESOLVED
    } else {
        "Next registration: automatic, at half the TTL (30 s at least)"
    };
    let r = status.releases;
    let releases = if r.releases == 0 {
        "Releases of BBMDs left: none asked".to_string()
    } else {
        format!(
            "Releases of BBMDs left: {} asked, {} released, {} refused, {} unanswered",
            r.releases, r.released, r.refused, r.unanswered
        )
    };
    for (area, text) in areas.into_iter().zip([sent, next.to_string(), releases]) {
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), area);
    }
}
