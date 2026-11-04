//! A B/IPv6 foreign device on the Network screen's panel (#2378). Its BBMD,
//! status and counts lines are B/IP's; this draws its status words and its
//! last three lines. B/IPv6 tracks no answers to its registrations, so the
//! panel counts what was sent and says that acceptance isn't known, where
//! B/IP shows the last BVLC-Result, a countdown and our FDT entry.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use bacnet_transport::bip6::Bip6ForeignStatus;

use super::{NEXT_ONCE_RESOLVED, Theme, WAITING, push_lookups};
use crate::tui::app::App;
use crate::tui::app::network::Registration;

/// The status words and their style, and the counters: registrations sent,
/// broadcasts refused while waiting (when any were), and the host name's
/// lookups with why the newest failed.
pub(super) fn status(
    app: &App,
    status: &Bip6ForeignStatus,
    theme: &Theme,
) -> (String, Style, String) {
    let (word, style) = match app.network.registration_state() {
        Some(Registration::Waiting) => (WAITING, theme.warn()),
        Some(Registration::Unconfirmed) => ("registering", theme.accent()),
        _ => ("first registration not sent yet", theme.dim()),
    };
    let mut counts = format!("registrations sent {}", status.registrations_sent);
    if status.broadcasts_refused > 0 {
        counts.push_str(&format!(
            "  broadcasts refused {}",
            status.broadcasts_refused
        ));
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
    (word.to_string(), style, counts)
}

/// The last three lines: what B/IPv6 can't show, when it registers next,
/// and the releases of the BBMDs it left.
pub(super) fn rows(frame: &mut Frame, areas: [Rect; 3], status: &Bip6ForeignStatus, theme: &Theme) {
    // As the state reads it: no address yet is waiting.
    let next = if status.waiting_for_bbmd || status.bbmd.is_none() {
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
    let lines = [
        "BBMD answers: not tracked on B/IPv6, so acceptance isn't shown".to_string(),
        next.to_string(),
        releases,
    ];
    for (area, text) in areas.into_iter().zip(lines) {
        frame.render_widget(Paragraph::new(Line::styled(text, theme.dim())), area);
    }
}
