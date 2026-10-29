//! Explicit scanner commands and identity-checked progress.
use super::monitor::MonitorTab;
use super::{App, FLASH_FOR, Link, Overlay};
use crate::tui::message::{Command, Failure};
use crate::tui::monitor::scan::{Scan, ScanCommand, ScanPhase, ScanReport};
use std::time::Duration;

impl App {
    pub(super) fn mstp_scan(&mut self, window: Option<Duration>, out: &mut Vec<Command>) {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            self.flash(
                "A baud scan needs a live receive-only MS/TP monitor.",
                FLASH_FOR,
            );
            return;
        };
        if window.is_none() {
            if let Some(scan) = monitor.scan.as_ref().filter(|s| s.active()) {
                out.push(Command::MonitorScan(ScanCommand::Cancel { id: scan.id }));
                self.flash(
                    "Cancelling scan; waiting for release, then restoring the original rate.",
                    FLASH_FOR,
                );
            } else {
                self.flash("No baud scan is running.", FLASH_FOR);
            }
            return;
        }
        if !monitor.live
            || monitor.scan.as_ref().is_some_and(|s| s.active())
            || self.mstp.station.is_some()
            || monitor.joining.is_some()
        {
            self.flash(
                "A baud scan needs an idle, live receive-only monitor.",
                FLASH_FOR,
            );
            return;
        }
        let Some(id) = self.next_op.checked_add(1) else {
            self.flash(
                "Operation identities exhausted; start a new session.",
                FLASH_FOR,
            );
            return;
        };
        self.next_op = id;
        let window = window.expect("start has an explicit window");
        monitor.scan = Some(Box::new(Scan::new(id, monitor.baud, window)));
        monitor.live = false;
        monitor.wizard = None;
        monitor.joining = None;
        monitor.shown = None;
        monitor.latest = None;
        monitor.baseline = None;
        monitor.tab = MonitorTab::Scan;
        monitor.scan_selected = 0;
        self.capture.wanted = false;
        self.overlay = Overlay::None;
        out.push(Command::MonitorScan(ScanCommand::Start { id, window }));
    }

    pub(super) fn scan_report(&mut self, report: ScanReport) {
        let Some(monitor) = self.monitor.as_deref_mut() else {
            return;
        };
        let Some(current) = monitor.scan.as_deref_mut() else {
            return;
        };
        match report {
            ScanReport::Progress(scan) if scan.id == current.id => {
                if current.active() && !scan.active() {
                    // Historical rows remain; resubscription requires a fresh
                    // screen entry after the scan has released its inputs.
                    self.capture.wanted = false;
                }
                if let ScanPhase::Stopped(error) = &scan.phase {
                    monitor.live = false;
                    self.link = Link::Failed(Failure::text(error.clone()));
                }
                monitor.scan = Some(scan);
            }
            ScanReport::Rejected { id, cancel, reason } if id == current.id => {
                if !cancel && current.phase == ScanPhase::Retiring {
                    current.phase = ScanPhase::Stopped(reason.clone());
                    // A coordinator refusal did not retire the original monitor.
                    monitor.live = matches!(self.link, Link::Passive);
                }
                self.flash(reason, FLASH_FOR);
            }
            _ => {}
        }
    }
}
