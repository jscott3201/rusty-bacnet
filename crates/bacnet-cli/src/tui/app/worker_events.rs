//! What the worker reports, and the commands it refused: the transitions
//! [`App`] makes for each [`WorkerEvent`], kept apart from `update`'s key
//! handling.

use super::*;
use crate::tui::point::{BrowseResult, ReqId};
use crate::tui::watched::WatchEvent;

impl App {
    /// Channel admission is part of the UI transition: never silently report
    /// a disconnected or connecting state for a command that was dropped.
    pub(crate) fn command_rejected(&mut self, command: Command, prior_link: Link, error: String) {
        self.link = prior_link;
        if matches!(command, Command::JoinTrunk { .. }) && self.monitor_join_refused(&error) {
            self.dirty = true;
            return;
        }
        // A name request is asked again on the next tick, without a notice.
        if let Command::Names { req, .. } = command {
            self.names.refused(req);
            return;
        }
        if let Command::Profile(_) = command {
            if let Some(connection) = &mut self.connection {
                connection.reply(Err(error.clone()));
            }
        }
        if let Command::WhoIs { op, .. } | Command::Cancel { op } = command {
            self.operation_rejected(op, matches!(command, Command::Cancel { .. }), error);
        } else {
            self.flash(error, FLASH_FOR);
        }
        self.dirty = true;
    }
    /// Only admission failure may correct an optimistic cancellation. Ordinary
    /// late progress/completion continues to require a running operation.
    fn operation_rejected(&mut self, id: OpId, cancel: bool, error: String) {
        let Some(op) = self.op.as_mut().filter(|op| {
            op.id == id
                && if cancel {
                    matches!(op.state, OpState::Cancelled | OpState::CancelFailed(_))
                } else {
                    op.running()
                }
        }) else {
            return;
        };
        if cancel {
            op.state = OpState::CancelFailed(error);
            self.ctrl_c_at = None;
            self.flash("Cancel refused. Ctrl-C to retry; x disconnects.", FLASH_FOR);
        } else {
            op.state = OpState::Failed(error.clone());
            self.flash(error, FLASH_FOR);
        }
    }

    /// A browser result: names go to the cache, from a name read or with
    /// the card or an object read; the rest to the browser. A card means
    /// the worker took the device read, which forgot the device's failed
    /// names, so the cache forgets them too.
    fn browse_result(&mut self, req: ReqId, result: BrowseResult) {
        let card = matches!(result, BrowseResult::Card(_));
        let free = match &result {
            BrowseResult::Names {
                device,
                address,
                names,
            } => {
                for (object, name) in names {
                    self.names.learn(*device, address, *object, name.clone());
                }
                return;
            }
            BrowseResult::Failed(_) if self.names.refused(req) => return,
            BrowseResult::Card(card) => card.name.clone().map(|name| (None, name)),
            BrowseResult::Object(view) => view.name.clone().map(|name| (Some(view.object), name)),
            _ => None,
        };
        let Some(browser) = self.browser.as_deref_mut() else {
            return;
        };
        let device = browser.device;
        // Only a result for this browser's device says whose name it is.
        if browser.apply(req, result) {
            let address = self
                .devices
                .get(device)
                .map_or("", |row| row.address.as_str());
            if card {
                self.names.forget_failures(device, address);
            }
            if let Some((object, name)) = free {
                if let Some(object) = object.or_else(|| names::device_object(device)) {
                    self.names.learn_name(device, address, object, name);
                }
            }
            self.settle_objects();
        }
    }

    pub(super) fn worker_event(&mut self, event: WorkerEvent, out: &mut Vec<Command>) {
        match event {
            WorkerEvent::Fatal { error } => self.fail(error, 1),
            WorkerEvent::Session { generation, event } => {
                if generation == self.generation && self.link != Link::Disconnecting {
                    self.worker_event(*event, out);
                }
            }
            WorkerEvent::SessionStarted { generation } => {
                self.generation = generation;
                self.monitor_joined();
                self.reset_session();
                if self.link != Link::Disconnecting {
                    self.link = Link::Connecting;
                }
                self.overlay = Overlay::None;
            }
            WorkerEvent::Disconnected { generation } => {
                if generation == self.generation {
                    self.reset_session();
                    self.link = Link::Down;
                    self.flash("Disconnected; local resources released.", FLASH_FOR);
                }
            }
            WorkerEvent::Profile(result) => {
                if let Err(error) = &result {
                    if self.link == Link::Connecting {
                        self.link = Link::Failed(Failure::text(error.clone()));
                    }
                }
                if let Some(connection) = &mut self.connection {
                    connection.reply(result);
                }
            }
            WorkerEvent::Rejected { error } if self.monitor_join_refused(&error) => {}
            WorkerEvent::Rejected { error } => {
                if self.link == Link::Connecting {
                    self.link = Link::Failed(Failure::text(error.clone()));
                }
                if let Some(op) = self.op.as_mut().filter(|op| op.running()) {
                    op.state = OpState::Failed(error.clone());
                }
                self.flash(error, FLASH_FOR);
            }
            WorkerEvent::Connected { local } => {
                self.link = Link::Up { local };
                self.start_capture(out);
            }
            // The footer shows its summary with the recovery keys; the
            // device screen and the log have the whole error.
            WorkerEvent::ConnectFailed { error } => self.link = Link::Failed(error),
            WorkerEvent::Discovered(row) | WorkerEvent::Updated(row) => {
                if let Some(op) = self.op.as_mut().filter(|op| op.running()) {
                    op.replies += usize::from(op.spec.wants(row.instance));
                }
                self.names.answered(row.instance, &row.address);
                self.devices.upsert(row);
                self.resolve_capture_devices();
            }
            WorkerEvent::Lost(row) => self.devices.remove(row.instance),
            WorkerEvent::Collision { retained, incoming } => {
                self.duplicates.record(&retained, &incoming);
            }
            WorkerEvent::Snapshot(rows) => {
                self.devices.replace_all(rows);
                self.resolve_capture_devices();
            }
            WorkerEvent::WhoIsSent { op } => {
                let now = self.now;
                if let Some(op) = self.running_op_mut(op) {
                    op.state = OpState::Listening {
                        until: now + op.spec.listen,
                    };
                }
            }
            WorkerEvent::OpRejected { op, cancel, error } => {
                self.operation_rejected(op, cancel, error)
            }
            WorkerEvent::Watch(event) => {
                if let WatchEvent::Details {
                    row,
                    name: Some(name),
                    ..
                } = &event
                {
                    let point = self
                        .watch
                        .rows()
                        .iter()
                        .find(|r| r.id == *row)
                        .map(|r| r.point);
                    if let Some(point) = point {
                        let address = self
                            .devices
                            .get(point.device)
                            .map_or("", |r| r.address.as_str());
                        self.names
                            .learn_name(point.device, address, point.object, name.clone());
                    }
                }
                if let Some(text) = self.watch.apply(event) {
                    self.flash(text, FLASH_FOR);
                }
            }
            WorkerEvent::Browse { req, result } => self.browse_result(req, result),
            WorkerEvent::OpFinished { op, outcome } => {
                if let Some(op) = self.running_op_mut(op) {
                    op.state = match outcome {
                        OpOutcome::Completed => OpState::Done,
                        OpOutcome::Cancelled => OpState::Cancelled,
                        OpOutcome::Failed(error) => OpState::Failed(error),
                    };
                }
            }
            WorkerEvent::Safety(event) => self.safety_event(event, out),
            WorkerEvent::Passive => {
                self.link = Link::Passive;
                // The monitor's session captures; a plain passive one can't.
                self.start_capture(out);
            }
            WorkerEvent::ForeignDevice(foreign) => self.network.foreign_device(foreign),
            WorkerEvent::Network(event) => self.network_event(event),
            WorkerEvent::Discovery(event) => self.discovery_event(event),
            WorkerEvent::Sc(report) => self.sc_report(report),
            WorkerEvent::Mstp(report) => self.mstp.report(report),
            WorkerEvent::Monitor(report) => self.monitor_report(report),
            WorkerEvent::Capture(event) => {
                if let Some(text) = self.capture.event(event) {
                    self.flash(text, FLASH_FOR);
                }
            }
        }
    }
}
