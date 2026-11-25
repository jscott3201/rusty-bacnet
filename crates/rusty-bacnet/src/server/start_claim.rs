//! What `start()` takes from a `BACnetServer`, and gives back when the
//! start is refused (#2748).
//!
//! At ownership transfer `start()` takes the pending registrations, the
//! save counters of the pending Notification Forwarders and the Audit
//! recipient, and freezes the forwarding configuration. All of it moves
//! into the startup future, which can still fail: a transport that cannot
//! be prepared, a router that has no port, a Device that cannot be built,
//! an SC dial that is refused, and then the Rust server's own start, such
//! as a port already in use or start-up validation. A [`StartClaim`] holds
//! what was taken, and its drop puts it back unless the objects have gone
//! into a running server, so the application can correct the fault and
//! start again. A refusal from the Rust server's start hands its database
//! back (#2752), and [`StartClaim::take_back`] takes the objects out of it.
//! The drop runs when the future is dropped, a cancelled start's on a
//! runtime thread some time after the cancel.
//!
//! What a cancel can't give back: a start cancelled once the objects are in
//! the Rust server's builder drops them with the builder's future, and the
//! objects do not clone. A B/IP, B/IPv6 or MS/TP start hands them over
//! without waiting, so a cancel that reaches it at all reaches the builder;
//! only an SC dial and a router's lookups wait before the hand-off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::sync::MutexExt;

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
    lock.locked()
}

/// A registration as the claim keeps it: the objects are `Send` already.
fn sendable(object: Box<dyn BACnetObject>) -> Box<dyn BACnetObject + Send> {
    object
}

/// Put `object` into `db`, asking first whether the database takes it
/// ([`ObjectDatabase::check_addable`], the checks its add makes), so a
/// refused object stays the claim's.
fn admit(
    db: &mut ObjectDatabase,
    object: Box<dyn BACnetObject + Send>,
) -> Result<(), (Error, Option<Box<dyn BACnetObject + Send>>)> {
    if let Err(error) = db.check_addable(object.as_ref()) {
        return Err((error, Some(object)));
    }
    db.add(object).map_err(|error| (error, None))
}

/// Let go of `objects` on the runtime's blocking pool when a runtime is
/// current, as `drop_database_off_runtime` lets go of a database: an object
/// with storage waits for its saves as it drops.
fn drop_off_runtime(objects: Objects) {
    if objects.is_empty() {
        return;
    }
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => drop(runtime.spawn_blocking(move || drop(objects))),
        Err(_) => drop(objects),
    }
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

    /// The database the start serves: the registrations in the order they
    /// were added, and the start's `device`, which Object_List puts first
    /// (#2669). Each was checked at its `add_*` call, so the database takes
    /// them all. Should it refuse one anyway, the claim keeps every
    /// registration, the refused one included, and gives them back as for a
    /// refusal before the hand-off (#2752). Once it is filled, only
    /// [`take_back`](Self::take_back) returns them.
    pub(super) fn fill_database(
        &mut self,
        device: Box<dyn BACnetObject + Send>,
    ) -> Result<ObjectDatabase, Error> {
        let mut db = ObjectDatabase::new();
        let mut pending = std::mem::take(&mut self.objects).into_iter();
        let mut refusal = None;
        for object in pending.by_ref() {
            if let Err(refused) = admit(&mut db, object) {
                refusal = Some(refused);
                break;
            }
        }
        let (error, kept) = match refusal {
            Some(refused) => refused,
            None => match admit(&mut db, device) {
                Ok(()) => {
                    self.handed_off = true;
                    return Ok(db);
                }
                Err((error, device)) => {
                    drop_off_runtime(device.into_iter().collect());
                    (error, None)
                }
            },
        };
        let mut back: Objects = db.into_objects().into_iter().map(sendable).collect();
        back.extend(kept);
        back.extend(pending);
        self.objects = back;
        Err(error)
    }

    /// Take the registrations back from `database`, which the Rust server's
    /// refused start handed back (#2752), leaving out the Device the start
    /// built: the claim then gives back everything it took, as for a refusal
    /// before the hand-off. They come back in the order they were added,
    /// which the database's Object_List keeps. The Device goes off the
    /// runtime, as the server lets go of a database.
    pub(super) fn take_back(&mut self, database: ObjectDatabase, device: ObjectIdentifier) {
        let (built, registered): (Objects, Objects) = database
            .into_objects()
            .into_iter()
            .map(sendable)
            .partition(|object| object.object_identifier() == device);
        drop_off_runtime(built);
        self.objects = registered;
        self.handed_off = false;
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
        claim_over(cells, vec![object(1), object(2)])
    }

    /// The start's Device, instance 9, named "Host".
    fn device() -> Box<dyn BACnetObject + Send> {
        use bacnet_objects::device::{DeviceConfig, DeviceObject};
        let config = DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(9, 0)
                .unwrap()
                .with_name("Host"),
            ..DeviceConfig::default()
        };
        Box::new(DeviceObject::new(config).unwrap())
    }

    /// Everything but the objects the claim took is back in the cells.
    fn given_back_whole(cells: &StartCells) {
        assert_eq!(relock(&cells.pending_forwarder_save_counters).len(), 1);
        assert!(relock(&cells.audit_recipient).is_some());
        assert!(
            !cells
                .forwarding_configuration_started
                .load(Ordering::Acquire)
        );
        assert!(!cells.start_in_flight.load(Ordering::Acquire));
    }

    /// A claim over `pending`, one forwarder's counters and a recipient.
    fn claim_over(cells: &StartCells, mut pending: Objects) -> StartClaim {
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
    fn a_claim_that_filled_its_database_gives_back_nothing_but_the_start() {
        let cells = cells();
        let mut claim = claim(&cells);
        assert_eq!(claim.fill_database(device()).unwrap().len(), 3);
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

    /// A database the Rust server's refused start hands back gives the
    /// registrations back in the order they were added, without the Device
    /// the start built, and with them everything else the claim took (#2752).
    #[test]
    fn objects_taken_back_from_a_refused_database_are_given_back_whole() {
        let cells = cells();
        let mut claim = claim(&cells);
        let db = claim.fill_database(device()).unwrap();
        claim.take_back(db, ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap());
        relock(&cells.pending_objects).push(object(3));
        drop(claim);
        assert_eq!(instances(&cells), [1, 2, 3]);
        given_back_whole(&cells);
    }

    /// A database that refuses a registration, which the `add_*` checks
    /// rule out, leaves every registration the claim's, the refused one
    /// included, in the order they were added; so does one that refuses
    /// the start's Device, which goes (#2752).
    #[test]
    fn a_registration_the_database_refuses_leaves_them_all_the_claims() {
        let named = |instance, name: &str| -> Box<dyn BACnetObject + Send> {
            Box::new(BinaryValueObject::new(instance, name).unwrap())
        };
        for (pending, expected) in [
            (vec![object(1), named(2, "BV-1"), object(3)], vec![1, 2, 3]),
            (vec![object(1), named(2, "Host")], vec![1, 2]),
        ] {
            let cells = cells();
            let mut claim = claim_over(&cells, pending);
            assert!(claim.fill_database(device()).is_err());
            drop(claim);
            assert_eq!(instances(&cells), expected);
            given_back_whole(&cells);
        }
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
