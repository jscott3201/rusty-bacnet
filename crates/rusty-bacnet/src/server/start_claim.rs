//! What `start()` takes from a `BACnetServer`, and gives back when the
//! start is refused (#2748).
//!
//! At ownership transfer `start()` takes the pending registrations, the
//! save counters of the pending Notification Forwarders and the Audit
//! recipient, and freezes the forwarding configuration. All of it moves
//! into the startup future, which can still fail: a transport that cannot
//! be prepared, a router that has no port, a Device that cannot be built,
//! or the application cancelling the start. A [`StartClaim`] holds what was
//! taken, and its drop puts it back unless the objects have gone into the
//! database, so the application can correct the fault and start again.
//!
//! Once the objects are in the database, a refusal from the Rust server's
//! own startup (a transport that will not bind, the SC dial) cannot give
//! them back: the database is consumed by the builder, and the objects do
//! not clone. Such a start leaves the server with no registrations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetRecipient;

use super::ForwarderSaveCounterEntry;

type Objects = Vec<Box<dyn BACnetObject + Send>>;

/// The server's shared cells a claim returns what it took to.
pub(super) struct StartCells {
    pub(super) pending_objects: Arc<Mutex<Objects>>,
    pub(super) pending_forwarder_save_counters: Arc<Mutex<Vec<ForwarderSaveCounterEntry>>>,
    pub(super) audit_recipient: Arc<Mutex<Option<BACnetRecipient>>>,
    pub(super) forwarding_configuration_started: Arc<AtomicBool>,
    pub(super) start_in_flight: Arc<AtomicBool>,
}

/// What one `start()` took. Dropping it ends the start: it gives back
/// what was taken if the objects never reached the database, and always
/// lets `add_*` and `start()` run again.
pub(super) struct StartClaim {
    cells: StartCells,
    objects: Objects,
    forwarder_save_counters: Vec<ForwarderSaveCounterEntry>,
    recipient: Option<BACnetRecipient>,
    forwarding_before: bool,
    handed_off: bool,
}

fn relock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl StartClaim {
    /// Take the registrations from `pending`, which the caller holds
    /// locked, and freeze the forwarding configuration. The caller took
    /// the save counters first, so no fallible step follows.
    pub(super) fn take(
        cells: StartCells,
        pending: &mut Objects,
        forwarder_save_counters: Vec<ForwarderSaveCounterEntry>,
        recipient: Option<BACnetRecipient>,
    ) -> Self {
        let forwarding_before = cells
            .forwarding_configuration_started
            .swap(true, Ordering::AcqRel);
        cells.start_in_flight.store(true, Ordering::Release);
        Self {
            objects: std::mem::take(pending),
            cells,
            forwarder_save_counters,
            recipient,
            forwarding_before,
            handed_off: false,
        }
    }

    /// The Audit recipient the Device is provisioned with.
    pub(super) fn recipient(&self) -> Option<&BACnetRecipient> {
        self.recipient.as_ref()
    }

    /// The registrations, for the database: from here a refusal cannot
    /// give them back, so nothing else is returned either.
    pub(super) fn hand_off_objects(&mut self) -> Objects {
        self.handed_off = true;
        std::mem::take(&mut self.objects)
    }

    /// The forwarders' save counters, for the running server.
    pub(super) fn take_forwarder_save_counters(&mut self) -> Vec<ForwarderSaveCounterEntry> {
        std::mem::take(&mut self.forwarder_save_counters)
    }
}

impl Drop for StartClaim {
    fn drop(&mut self) {
        let mut pending = relock(&self.cells.pending_objects);
        if !self.handed_off {
            let mut returned = std::mem::take(&mut self.objects);
            returned.append(&mut pending);
            *pending = returned;
            let mut counters = relock(&self.cells.pending_forwarder_save_counters);
            let mut returned = std::mem::take(&mut self.forwarder_save_counters);
            returned.append(&mut counters);
            *counters = returned;
            *relock(&self.cells.audit_recipient) = self.recipient.take();
            self.cells
                .forwarding_configuration_started
                .store(self.forwarding_before, Ordering::Release);
        }
        self.cells.start_in_flight.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_objects::notification_forwarder::NotificationForwarderObject;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    fn cells() -> StartCells {
        StartCells {
            pending_objects: Arc::new(Mutex::new(Vec::new())),
            pending_forwarder_save_counters: Arc::new(Mutex::new(Vec::new())),
            audit_recipient: Arc::new(Mutex::new(None)),
            forwarding_configuration_started: Arc::new(AtomicBool::new(false)),
            start_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    fn object(instance: u32) -> Box<dyn BACnetObject + Send> {
        Box::new(BinaryValueObject::new(instance, format!("BV-{instance}")).unwrap())
    }

    fn counters(instance: u32) -> ForwarderSaveCounterEntry {
        let nf = NotificationForwarderObject::new(instance, "NF").unwrap();
        (instance, nf.save_counters())
    }

    fn recipient() -> BACnetRecipient {
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap())
    }

    /// A claim over two objects, one forwarder's counters and a recipient.
    fn claim(cells: &StartCells) -> StartClaim {
        let mut pending = vec![object(1), object(2)];
        StartClaim::take(
            StartCells {
                pending_objects: Arc::clone(&cells.pending_objects),
                pending_forwarder_save_counters: Arc::clone(&cells.pending_forwarder_save_counters),
                audit_recipient: Arc::clone(&cells.audit_recipient),
                forwarding_configuration_started: Arc::clone(
                    &cells.forwarding_configuration_started,
                ),
                start_in_flight: Arc::clone(&cells.start_in_flight),
            },
            &mut pending,
            vec![counters(7)],
            Some(recipient()),
        )
    }

    fn instances(cells: &StartCells) -> Vec<u32> {
        relock(&cells.pending_objects)
            .iter()
            .map(|object| object.object_identifier().instance_number())
            .collect()
    }

    #[test]
    fn a_dropped_claim_gives_back_everything_it_took_ahead_of_newer_objects() {
        let cells = cells();
        let claim = claim(&cells);
        assert!(cells.start_in_flight.load(Ordering::Acquire));
        assert!(
            cells
                .forwarding_configuration_started
                .load(Ordering::Acquire)
        );
        assert!(relock(&cells.pending_objects).is_empty());
        relock(&cells.pending_objects).push(object(3));
        drop(claim);
        assert_eq!(instances(&cells), [1, 2, 3]);
        assert_eq!(relock(&cells.pending_forwarder_save_counters).len(), 1);
        assert!(relock(&cells.audit_recipient).is_some());
        assert!(
            !cells
                .forwarding_configuration_started
                .load(Ordering::Acquire)
        );
        assert!(!cells.start_in_flight.load(Ordering::Acquire));
    }

    #[test]
    fn a_claim_that_handed_off_its_objects_gives_back_nothing_but_the_start() {
        let cells = cells();
        let mut claim = claim(&cells);
        assert_eq!(claim.hand_off_objects().len(), 2);
        assert_eq!(claim.take_forwarder_save_counters().len(), 1);
        drop(claim);
        assert!(instances(&cells).is_empty());
        assert!(relock(&cells.pending_forwarder_save_counters).is_empty());
        assert!(relock(&cells.audit_recipient).is_none());
        // The configuration stays frozen, and the next call may run.
        assert!(
            cells
                .forwarding_configuration_started
                .load(Ordering::Acquire)
        );
        assert!(!cells.start_in_flight.load(Ordering::Acquire));
    }

    #[test]
    fn a_refused_start_keeps_a_freeze_an_earlier_start_set() {
        let cells = cells();
        cells
            .forwarding_configuration_started
            .store(true, Ordering::Release);
        drop(claim(&cells));
        assert!(
            cells
                .forwarding_configuration_started
                .load(Ordering::Acquire)
        );
        assert_eq!(instances(&cells), [1, 2]);
    }
}
