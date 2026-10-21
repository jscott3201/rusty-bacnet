//! Who-Has and router discovery in the UI (#960).
//!
//! - `:find NAME` broadcasts a Who-Has; the I-Haves that name the object
//!   fill a panel below the device table, one row per device and object.
//!   `:find` alone closes it.
//! - The Network screen's Networks tab maps routers to the networks they
//!   reach: every I-Am-Router-To-Network heard adds to it, and `r` there
//!   clears it and asks every router again.
//! - A router's Reject-Message-To-Network is flashed, naming the network.

use std::time::Instant;

use super::network::View;
use super::watch::Screen;
use super::{App, Link, FLASH_FOR};
use crate::tui::message::Command;
use crate::tui::network::discovery::{DiscoveryCommand, DiscoveryEvent, FoundRow};
use crate::tui::network::routers::RouterMap;

/// The running or last Who-Has search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhoHasSearch {
    /// The object name searched for.
    pub(crate) name: String,
    /// When the Who-Has went.
    pub(crate) sent: Instant,
    /// The objects found, in the order their I-Haves arrived.
    pub(crate) found: Vec<FoundRow>,
    /// Why the Who-Has could not be sent.
    pub(crate) failed: Option<String>,
}

impl WhoHasSearch {
    /// Keep `row` if it names the object searched for, once per device and
    /// object. Returns whether it was kept.
    pub(crate) fn accept(&mut self, row: FoundRow) -> bool {
        let seen = self
            .found
            .iter()
            .any(|kept| kept.device == row.device && kept.object == row.object);
        if seen || row.name != self.name {
            return false;
        }
        self.found.push(row);
        true
    }
}

impl App {
    /// `:find NAME`: search for the object, or with no name close the
    /// search panel.
    pub(super) fn find(&mut self, name: String, out: &mut Vec<Command>) {
        if name.is_empty() {
            self.who_has = None;
            return;
        }
        if !self.discovery_ready() {
            return;
        }
        self.who_has = Some(WhoHasSearch {
            name: name.clone(),
            sent: self.now,
            found: Vec::new(),
            failed: None,
        });
        self.screen = Screen::Main;
        out.push(Command::Discovery(DiscoveryCommand::WhoHas { name }));
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
                        if let Some(search) = self.who_has.as_mut().filter(|s| s.name == name) {
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
