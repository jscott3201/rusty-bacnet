//! Physical release, abort and admission, independently of core completion.

use bacnet_mstp_core::TransmitId;

use super::{Event, Sim};
use crate::event::Class;
use crate::line::{Driver, TxId};
use crate::report::{DriverRelease, DriverReleaseCause, ViolationKind};

impl Sim {
    /// A release belongs to one monotonic line transmission ID, never just
    /// to a station. A power failure removes it, so an old queued release
    /// cannot clear a later transmission or re-enable an unpowered receiver.
    pub(super) fn release_driver(&mut self, tx: TxId, cause: DriverReleaseCause) {
        let Some(owner) = self.line.release(tx) else {
            return;
        };
        self.meta.remove(&tx);
        self.report.driver_releases.push(DriverRelease {
            driver: owner.driver,
            start: owner.start,
            data_end: owner.data_complete.then_some(owner.data_end),
            scheduled: owner.release,
            at: self.now,
            cause,
        });
        if let Driver::Station(station) = owner.driver {
            let node = self.index_of(station);
            if self.nodes[node].driving == Some(tx) {
                self.nodes[node].driving = None;
                if cause != DriverReleaseCause::PowerLoss {
                    self.nodes[node].listening_since = Some(self.now);
                }
            }
        }
    }

    /// Receive while a core operation is still outstanding means that the
    /// core withdrew it. Normal completion takes `transmitting` before the
    /// callback, so it cannot accidentally end a completed post-drive hold.
    pub(super) fn abort_withdrawn_transmission(&mut self, node: usize) {
        let master = self.master_mut(node).expect("a master");
        let Some(id) = master.transmitting else {
            return;
        };
        if master
            .core
            .as_ref()
            .is_some_and(|core| core.snapshot().transmit_id == Some(id))
        {
            return;
        }
        master.transmitting = None;
        let Some(tx) = self.nodes[node].driving else {
            return;
        };
        if !self
            .meta
            .get(&tx)
            .is_some_and(|meta| meta.transmit_id == Some(id))
        {
            self.violation(
                ViolationKind::ContractBroken,
                "withdrawn core transmit does not match physical ownership".into(),
            );
            return;
        }
        self.perturb("an octet transmit watchdog expired");
        if let Some(transmission) = self.line.truncate(tx, self.now) {
            self.record_frame(tx, transmission);
        }
        self.release_driver(tx, DriverReleaseCause::Watchdog);
    }

    /// Even when software completion already happened, admission cannot
    /// confirm a new start until this adapter's old physical driver releases.
    pub(super) fn wait_for_release(&mut self, node: usize, id: TransmitId) -> bool {
        let Some(owner) = self.nodes[node].driving.and_then(|tx| self.line.owner(tx)) else {
            return false;
        };
        if owner.release <= self.now {
            return false;
        }
        let release = owner.release;
        let station = self.nodes[node].station;
        let Some(master) = self.master_mut(node) else {
            return true;
        };
        if master
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            let incarnation = master.incarnation;
            self.push(
                release,
                Class::AdapterStart,
                station,
                Event::AdapterStart {
                    node,
                    id,
                    incarnation,
                },
            );
        }
        true
    }
}
