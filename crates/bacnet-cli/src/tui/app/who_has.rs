//! Who-Has and router discovery in the UI (#960).
//!
//! - `:find NAME` broadcasts a Who-Has in an active session, or filters
//!   retained/live observations in a listening passive session. Results
//!   distinguish device, object and station address.
//!   `:find` alone closes it.
//! - The Network screen's Networks tab maps routers to the networks they
//!   reach: every I-Am-Router-To-Network heard adds to it, and `r` there
//!   clears it and asks every router again.
//! - A router's Reject-Message-To-Network is flashed, naming the network.

use std::time::Instant;

use super::network::View;
use super::watch::Screen;
use super::{App, FLASH_FOR, Link};
use crate::tui::message::Command;
use crate::tui::network::discovery::{DiscoveryCommand, DiscoveryEvent, FoundRow};
use crate::tui::network::routers::RouterMap;

/// Maximum I-Have observations and search results retained per session.
pub(crate) const MAX_OBSERVATIONS: usize = 4_096;
/// Additional aggregate string-byte bound on each observation/result collection.
pub(crate) const MAX_OBSERVATION_BYTES: usize = 4 * 1024 * 1024;

/// Bounded most-recent observations; repeats refresh their place and name.
#[derive(Debug, Default)]
pub(crate) struct Observations(pub(crate) Vec<FoundRow>);
impl Observations {
    pub(crate) fn record(&mut self, row: FoundRow) {
        retain_row(&mut self.0, row);
    }
}
fn same_station_object(a: &FoundRow, b: &FoundRow) -> bool {
    a.device == b.device
        && a.object == b.object
        && a.address == b.address
        && a.via == b.via
        && a.network == b.network
}
fn row_bytes(row: &FoundRow) -> usize {
    row.name.len() + row.address.len() + row.via.as_ref().map_or(0, String::len)
}
fn retain_row(rows: &mut Vec<FoundRow>, row: FoundRow) -> bool {
    let bytes = row_bytes(&row);
    if bytes > MAX_OBSERVATION_BYTES {
        return false;
    }
    if let Some(index) = rows.iter().position(|kept| same_station_object(kept, &row)) {
        rows.remove(index);
    }
    let mut retained: usize = rows.iter().map(row_bytes).sum();
    while rows.len() >= MAX_OBSERVATIONS || retained + bytes > MAX_OBSERVATION_BYTES {
        retained -= row_bytes(&rows.remove(0));
    }
    rows.push(row);
    true
}

/// The running or last Who-Has search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhoHasSearch {
    /// The object name searched for.
    pub(crate) name: String,
    /// When the Who-Has went.
    pub(crate) sent: Instant,
    /// This search filters overheard observations without sending.
    pub(crate) passive: bool,
    /// The objects found, in the order their I-Haves arrived.
    pub(crate) found: Vec<FoundRow>,
    /// Why the Who-Has could not be sent.
    pub(crate) failed: Option<String>,
}

impl WhoHasSearch {
    /// Refresh a matching station/object, evicting oldest rows within the count
    /// and byte limits. A renamed observation removes the obsolete match.
    /// Returns whether a new matching identity was kept.
    pub(crate) fn accept(&mut self, row: FoundRow) -> bool {
        if row.name != self.name {
            self.found.retain(|kept| !same_station_object(kept, &row));
            return false;
        }
        let fresh = !self
            .found
            .iter()
            .any(|kept| same_station_object(kept, &row));
        retain_row(&mut self.found, row) && fresh
    }
}

impl App {
    /// A bound passive listener is still supplying observations.
    pub(crate) fn passive_listening(&self) -> bool {
        self.passive_listener.is_some() && self.link == Link::Passive
    }

    /// `:find NAME`: search for the object, or with no name close the
    /// search panel.
    pub(super) fn find(&mut self, name: String, out: &mut Vec<Command>) {
        if name.is_empty() {
            self.who_has = None;
            return;
        }
        let passive = self.passive_listening();
        if !passive && !self.discovery_ready() {
            return;
        }
        self.who_has = Some(WhoHasSearch {
            name: name.clone(),
            sent: self.now,
            passive,
            found: Vec::new(),
            failed: None,
        });
        self.screen = Screen::Main;
        if passive {
            let search = self.who_has.as_mut().unwrap();
            for row in &self.overheard.0 {
                search.accept(row.clone());
            }
        } else {
            out.push(Command::Discovery(DiscoveryCommand::WhoHas { name }));
        }
    }

    /// `r` on the Networks tab: ask every router for its networks.
    pub(super) fn ask_routers(&mut self, out: &mut Vec<Command>) {
        if self.discovery_ready() {
            // A newer announcement supersedes an older one (Clause
            // 6.6.3.2): a router replaced or moved since the last search
            // must not stay flagged, so each search starts a fresh map.
            self.network.routers = RouterMap::default();
            self.network.routers_asked = Some(self.now);
            out.push(Command::Discovery(DiscoveryCommand::WhoIsRouter));
        }
    }

    fn discovery_ready(&mut self) -> bool {
        if !self.may_transmit() {
            return false;
        }
        if !matches!(self.link, Link::Up { .. }) {
            self.flash("Not connected yet.", FLASH_FOR);
            return false;
        }
        true
    }

    /// A discovery report.
    pub(super) fn discovery_event(&mut self, event: DiscoveryEvent) {
        let now = self.now;
        match event {
            DiscoveryEvent::IHave(row) => {
                // Every I-Have names an object, asked for or not.
                self.names
                    .learn_name(row.device, &row.address, row.object, row.name.clone());
                if self.passive_listening() {
                    self.overheard.record(row.clone());
                }
                if let Some(search) = &mut self.who_has {
                    search.accept(row);
                }
            }
            DiscoveryEvent::Router { router, networks } => {
                self.network.routers.record(&router, &networks, now);
            }
            DiscoveryEvent::NetworkNumber {
                network,
                configured,
            } => self.network.local_number = Some((network, configured)),
            DiscoveryEvent::Rejected {
                router,
                network,
                reason,
            } => self.flash(
                format!("Router {router} rejected a message for network {network}: {reason}"),
                FLASH_FOR,
            ),
            DiscoveryEvent::Refused { command, reason } => {
                match command {
                    DiscoveryCommand::WhoHas { name } => {
                        if let Some(search) = self
                            .who_has
                            .as_mut()
                            .filter(|s| !s.passive && s.name == name)
                        {
                            search.failed = Some(reason.clone());
                        }
                    }
                    DiscoveryCommand::WhoIsRouter => self.network.routers_asked = None,
                }
                self.flash(reason, FLASH_FOR);
            }
        }
    }

    /// Whether the Network screen shows the Networks tab.
    pub(crate) fn networks_tab(&self) -> bool {
        self.screen == Screen::Network && self.network.view == View::Networks
    }
}
