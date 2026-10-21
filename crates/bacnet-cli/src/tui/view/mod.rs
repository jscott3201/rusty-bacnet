//! Rendering: pure functions of `&App` (apart from the table's scroll
//! position, which the table keeps in a `Cell` between frames).
//!
//! 80x24 is the smallest supported terminal and 120x40 the full layout.
//! Below the minimum only a notice is drawn.

mod bars;
mod browser;
mod capture;
mod chrome;
mod columns;
mod connection;
mod devices;
mod dialogs;
mod fit;
mod help;
mod monitor;
mod mstp;
mod network;
mod networks;
mod safety;
mod sc;
mod sc_credentials;
mod watch;
mod who_has;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::app::names::Name;
use super::app::watch::Screen;
use super::app::{App, Link, Overlay};

/// Smallest usable width.
pub(crate) const MIN_WIDTH: u16 = 80;
/// Smallest usable height.
pub(crate) const MIN_HEIGHT: u16 = 24;
/// Width from which the wide table columns are used.
pub(crate) const FULL_WIDTH: u16 = 120;
/// Width from which panes that stack at the full width sit side by side.
pub(crate) const WIDE_WIDTH: u16 = 160;
/// Height from which the log pane gets more room.
pub(crate) const FULL_HEIGHT: u16 = 40;

/// Draw one frame.
pub(crate) fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let theme = Theme { color: app.color };
    // The tables that draw with names say so; a table not drawn asks for none.
    app.devices.named.set(false);
    if let Some(browser) = &app.browser {
        browser.objects.named.set(false);
    }
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small(frame, area, &theme);
        return;
    }
    let mut banners = app.duplicates.banner().to_vec();
    banners.extend(app.network.registration_banner());
    // Wrapped here, so the height is exactly what is drawn.
    let banner: Vec<String> = banners
        .iter()
        .flat_map(|text| devices::wrap_banner(text, usize::from(area.width)))
        .collect();
    let banner_height = banner.len();
    let log_height = match (app.show_log, area.height >= FULL_HEIGHT) {
        (false, _) => 0,
        (true, true) => 12,
        (true, false) => 7,
    };
    let [status, banner_area, body, log, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(banner_height as u16),
        Constraint::Min(5),
        Constraint::Length(log_height),
        Constraint::Length(1),
    ])
    .areas(area);
    bars::status_bar(frame, status, app, &theme);
    devices::banner(frame, banner_area, &banner, &theme);
    match &app.browser {
        _ if app.screen == Screen::Watch => watch::draw(frame, body, app, &theme),
        _ if app.screen == Screen::Network => network::draw(frame, body, app, &theme),
        _ if app.screen == Screen::Sc => sc::draw(frame, body, app, &theme),
        _ if app.screen == Screen::Mstp => mstp::draw(frame, body, app, &theme),
        _ if app.screen == Screen::Capture => capture::draw(frame, body, app, &theme),
        Some(open) if app.screen == Screen::Browse => {
            browser::draw(frame, body, open, Names::of(app), &theme)
        }
        _ => who_has::devices(frame, body, app, &theme),
    }
    if app.show_log {
        chrome::log_pane(frame, log, app, &theme);
    }
    let tab_focus = app.screen == Screen::Network && network::wide(app, body);
    bars::footer(frame, footer, app, &theme, tab_focus);
    match &app.overlay {
        // The command bar is the footer.
        Overlay::None | Overlay::Command(_) => {}
        Overlay::Connection => {
            if let Some(connection) = &app.connection {
                let live = !matches!(app.link, Link::Down | Link::Failed(_));
                connection::draw(frame, area, connection, live, &theme);
            }
        }
        Overlay::Help => help::draw(frame, area, app, &theme),
        Overlay::WhoIs(form) => dialogs::who_is(frame, area, app, form, &theme),
        Overlay::Picker(picker) => dialogs::picker(frame, area, picker, &theme),
        Overlay::Safety(modal) => safety::draw(frame, area, app, modal, &theme),
        Overlay::Join(dialog) => mstp::join(frame, area, dialog, &app.mstp_thread, &theme),
    }
}

fn too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    let text = vec![
        Line::styled(
            format!("Terminal too small: {}x{}.", area.width, area.height),
            theme.warn(),
        ),
        Line::raw(format!(
            "bacnet tui needs at least {MIN_WIDTH}x{MIN_HEIGHT} \
             ({FULL_WIDTH}x{FULL_HEIGHT} for the full layout)."
        )),
        // Not "q": it types into an open form or filter.
        Line::raw("Resize the window, or press Ctrl-C twice to quit."),
    ];
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
}

/// Styles, with colour only when `NO_COLOR` is unset. Without colour every
/// state is still marked by text or a modifier.
pub(crate) struct Theme {
    color: bool,
}

impl Theme {
    fn pick(&self, colored: Style, plain: Style) -> Style {
        if self.color {
            colored
        } else {
            plain
        }
    }

    /// Status bar.
    fn bar(&self) -> Style {
        self.pick(
            Style::new().fg(Color::Black).bg(Color::Cyan),
            Style::new().add_modifier(Modifier::REVERSED),
        )
    }

    /// The READ-ONLY badge.
    fn safe(&self) -> Style {
        self.pick(
            Style::new()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::new().add_modifier(Modifier::BOLD),
        )
    }

    /// Headings and focused labels.
    fn accent(&self) -> Style {
        self.pick(
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            Style::new().add_modifier(Modifier::BOLD),
        )
    }

    /// Warnings.
    fn warn(&self) -> Style {
        self.pick(
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            Style::new().add_modifier(Modifier::BOLD),
        )
    }

    /// Errors and the duplicate banner.
    fn danger(&self) -> Style {
        self.pick(
            Style::new()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
            Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
    }

    /// Secondary text.
    fn dim(&self) -> Style {
        self.pick(
            Style::new().fg(Color::DarkGray),
            Style::new().add_modifier(Modifier::DIM),
        )
    }

    /// Selected row or focused value.
    fn selected(&self) -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }
}

/// `text` with each control character (a newline, a tab, an escape) as a
/// space, as the log pane shows them: a device-supplied string must not
/// move the cursor or break a row.
pub(super) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// Narrowest a name column is drawn; with less room it is left out, and
/// its names aren't asked for (#1896).
pub(super) const MIN_NAME: usize = 8;

/// The session's names as the tables show them.
#[derive(Clone, Copy)]
pub(super) struct Names<'a> {
    app: &'a App,
    /// Names not yet known are being asked for.
    live: bool,
}

impl<'a> Names<'a> {
    /// The app's names.
    pub(super) fn of(app: &'a App) -> Self {
        Self {
            app,
            live: app.names_live(),
        }
    }

    /// `object`'s name on `device` (at the address the device table
    /// shows) as a cell shows it: the name (already free of control
    /// characters), `…` while it is being asked for, `-` when the device
    /// refused it, `?` when it didn't answer (until its next I-Am), and
    /// nothing when nothing can be asked.
    pub(super) fn text(
        &self,
        device: u32,
        object: Option<bacnet_types::primitives::ObjectIdentifier>,
    ) -> &'a str {
        match object.and_then(|object| self.app.name_of(device, object)) {
            Some(Name::Known(name)) => name,
            Some(Name::Unavailable) => "-",
            Some(Name::Unreachable) => UNREAD,
            None if self.live => PENDING,
            None => "",
        }
    }
}

/// A name cell while the name is being asked for.
pub(super) const PENDING: &str = "…";

/// A name cell when the device didn't answer for it (#1931); drawn dim,
/// like [`PENDING`], so it stands apart from a name.
pub(super) const UNREAD: &str = "?";

/// True for the name cells drawn dim: a marker, not a name.
pub(super) fn name_marker(text: &str) -> bool {
    text == PENDING || text == UNREAD
}

/// Display width of `text`.
pub(super) fn text_width(text: &str) -> usize {
    ratatui::text::Span::raw(text).width()
}

/// `text` cut to at most `width` columns, ending in `…` when it was cut.
pub(super) fn ellipsize(text: &str, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if text_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let w = text_width(grapheme);
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push_str(grapheme);
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// `lines` in `height` rows: when they don't all fit, the last row shown
/// says how many more there are (`+N more`).
pub(super) fn more(
    mut lines: Vec<Line<'static>>,
    height: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let height = usize::from(height).max(1);
    if lines.len() > height {
        let hidden = lines.len() - (height - 1);
        lines.truncate(height - 1);
        lines.push(Line::styled(format!("+{hidden} more"), theme.warn()));
    }
    lines
}

/// A `width` x `height` rectangle centred in `area`, clipped to it.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    area.centered(
        Constraint::Length(width.min(area.width)),
        Constraint::Length(height.min(area.height)),
    )
}
