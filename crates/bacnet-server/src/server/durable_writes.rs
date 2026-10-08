//! Staging writes whose new state an object saves first (#1270).
//!
//! A Notification Forwarder saves a written Recipient_List or
//! Subscribed_Recipients, a Notification Class a written Recipient_List
//! (#1315), an Access Rights object a written rule array, Enable (#1392) or
//! Accompaniment (#1393), and an Audit Log a Log_Enable or Buffer_Size
//! change, before serving it, and refuses the write if the save fails. So
//! that the save never runs while the database guard is held, a request
//! that makes such a write stages it first ([`DurableWrites`]): under the
//! guard the object queues the save, the request awaits it with the guard
//! dropped, and then runs as it always has, the object taking the saved
//! state or refusing the write. The request releases what it staged in the
//! critical section that makes the write. An application's Audit Log purge (#1238) is staged the
//! same way.
//!
//! A request stages once per object, handing it all of the request's writes
//! to it in order, and the object folds them into one save (#1423): a
//! WritePropertyMultiple that provisions an Access Rights object saves its
//! rule arrays and Enable together, off the guard.
//!
//! Under a mutation authorizer, a WritePropertyMultiple decides each attempt
//! it would stage before staging it, and stages only those allowed (#1321).
//! The handler then answers each such attempt from the decision made ahead
//! instead of asking again (`mutations::wpm_ahead`).
//!
//! Other requests read and write the database while the save runs. One that
//! stages a write to the same object waits for the first to land; requests
//! that stage writes to several objects stage them in object order, so two
//! of them never wait on each other.
//!
//! [`DurableWrites`]: bacnet_objects::durable::DurableWrites

use std::sync::Arc;
use std::time::Duration;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::durable::{PendingWrite, SaveWait, StageStep};
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::wpm::{
    WritePropertyAttempt, WritePropertyMultipleCursor, WritePropertyMultipleEvent,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::Bytes;
use tokio::sync::RwLock;

use crate::handlers;

/// How long a request that found an object busy waits before it stages
/// again. The object drops a staged write its request has forgotten after a
/// while, so a retry gets through even then.
pub(super) const BUSY_RECHECK: Duration = Duration::from_secs(1);

/// One change a request is about to make to an object that may save it.
pub(super) struct DurableTarget {
    oid: ObjectIdentifier,
    change: Change,
}

enum Change {
    /// A property write.
    Write {
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: TargetValue,
    },
    /// The application's purge of the object's records.
    Purge,
}

enum TargetValue {
    /// The decoded written value, localized under the guard as the handler
    /// localizes it.
    Written(PropertyValue),
    /// The list an AddListElement (`remove` false) or RemoveListElement
    /// leaves, computed under the guard from the stored list.
    ListEdit { service_data: Bytes, remove: bool },
}

/// Whether a bundled object of `oid`'s type may save a write of `property`
/// first. A forwarder saves its Recipient_List and Subscribed_Recipients, a
/// Notification Class its Recipient_List, an Audit Log its Log_Enable and
/// Buffer_Size, and an Access Rights object its two rule arrays, Enable
/// (property 133, named `LOG_ENABLE`) and Accompaniment. Under an
/// authorizer, only these attempts of a WritePropertyMultiple are decided
/// ahead of the handler (#1321).
///
/// An object type that takes up [`DurableWrites`] is listed here too, or
/// the server never stages its writes and they save in place under the
/// guard (see "Adding an object" in [`bacnet_objects::durable`]).
///
/// [`DurableWrites`]: bacnet_objects::durable::DurableWrites
fn may_save(oid: ObjectIdentifier, property: PropertyIdentifier) -> bool {
    match oid.object_type() {
        ObjectType::NOTIFICATION_FORWARDER => matches!(
            property,
            PropertyIdentifier::RECIPIENT_LIST | PropertyIdentifier::SUBSCRIBED_RECIPIENTS
        ),
        ObjectType::AUDIT_LOG => matches!(
            property,
            PropertyIdentifier::LOG_ENABLE | PropertyIdentifier::BUFFER_SIZE
        ),
        ObjectType::NOTIFICATION_CLASS => property == PropertyIdentifier::RECIPIENT_LIST,
        ObjectType::COLOR
        | ObjectType::COLOR_TEMPERATURE
        | ObjectType::LIGHTING_OUTPUT
        | ObjectType::BINARY_LIGHTING_OUTPUT
        | ObjectType::ANALOG_VALUE
        | ObjectType::BINARY_VALUE
        | ObjectType::MULTI_STATE_VALUE
        | ObjectType::ANALOG_INPUT
        | ObjectType::BINARY_INPUT
        | ObjectType::MULTI_STATE_INPUT => property == PropertyIdentifier::TAGS,
        ObjectType::ACCESS_RIGHTS => matches!(
            property,
            PropertyIdentifier::POSITIVE_ACCESS_RULES
                | PropertyIdentifier::NEGATIVE_ACCESS_RULES
                | PropertyIdentifier::LOG_ENABLE
                | PropertyIdentifier::ACCOMPANIMENT
        ),
        _ => false,
    }
}

/// Whether a bundled object of `oid`'s type holds `property` as a
/// BACnetLIST. No type [`may_save`] admits departs from the standard
/// classification, so a value decodes here as the handler decodes it after
/// asking the object.
fn held_as_list(oid: ObjectIdentifier, property: PropertyIdentifier) -> bool {
    bacnet_objects::traits::standard_list_property(oid.object_type(), property)
}

impl DurableTarget {
    /// The write a WriteProperty request makes, if its object may save it.
    pub(super) fn write_property(service_data: &[u8]) -> Vec<Self> {
        let Ok(request) = WritePropertyRequest::decode(service_data) else {
            return Vec::new();
        };
        if !may_save(request.object_identifier, request.property_identifier) {
            return Vec::new();
        }
        let Ok(value) = handlers::decode_write_property_value(
            request.property_identifier,
            request.property_array_index,
            held_as_list(request.object_identifier, request.property_identifier),
            &request.property_value,
        ) else {
            return Vec::new();
        };
        vec![Self::write(
            request.object_identifier,
            request.property_identifier,
            request.property_array_index,
            TargetValue::Written(value),
        )]
    }

    fn write(
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: TargetValue,
    ) -> Self {
        Self {
            oid,
            change: Change::Write {
                property,
                array_index,
                value,
            },
        }
    }

    /// The attempts of a WritePropertyMultiple request on objects that may
    /// save them, in wire order, each with the write it makes. They end at
    /// the first such attempt whose value doesn't decode, since the request
    /// stops there, as it does at an undecodable suffix.
    pub(super) fn write_property_multiple(
        service_data: &[u8],
    ) -> Vec<(WritePropertyAttempt, Self)> {
        let mut cursor = WritePropertyMultipleCursor::new(service_data);
        let mut attempts = Vec::new();
        while let Ok(Some(event)) = cursor.next_event() {
            let WritePropertyMultipleEvent::WriteAttempt(attempt) = event else {
                continue;
            };
            let reference = &attempt.reference;
            let oid = reference.object_identifier;
            let property = PropertyIdentifier::from_raw(reference.property_identifier);
            if !may_save(oid, property) {
                continue;
            }
            let Ok(value) = handlers::decode_write_property_value(
                property,
                reference.property_array_index,
                held_as_list(oid, property),
                &attempt.value,
            ) else {
                break;
            };
            let index = reference.property_array_index;
            let target = Self::write(oid, property, index, TargetValue::Written(value));
            attempts.push((attempt, target));
        }
        attempts
    }

    /// The writes of `attempts` to stage, in object order and then request
    /// order; [`stage`] hands each object its writes together. `allowed`
    /// decides each attempt in turn before it is staged (#1321). The request
    /// stops at one it denies, so neither that attempt nor any after it is
    /// staged or decided.
    pub(super) fn in_object_order(
        attempts: Vec<(WritePropertyAttempt, Self)>,
        mut allowed: impl FnMut(&WritePropertyAttempt) -> bool,
    ) -> Vec<Self> {
        let mut targets: Vec<Self> = attempts
            .into_iter()
            .map_while(|(attempt, target)| allowed(&attempt).then_some(target))
            .collect();
        targets.sort_by_key(|target| {
            (
                target.oid.object_type().to_raw(),
                target.oid.instance_number(),
            )
        });
        targets
    }

    /// The write an AddListElement or RemoveListElement request makes, if
    /// its object may save it.
    pub(super) fn list_element(service_data: &Bytes, remove: bool) -> Vec<Self> {
        let Ok(request) = ListElementRequest::decode(service_data) else {
            return Vec::new();
        };
        if !may_save(request.object_identifier, request.property_identifier) {
            return Vec::new();
        }
        vec![Self::write(
            request.object_identifier,
            request.property_identifier,
            request.property_array_index,
            TargetValue::ListEdit {
                service_data: service_data.clone(),
                remove,
            },
        )]
    }

    /// A local property write, if its object may save it.
    pub(super) fn local(
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> Vec<Self> {
        if !may_save(oid, property) {
            return Vec::new();
        }
        vec![Self::write(
            oid,
            property,
            array_index,
            TargetValue::Written(value.clone()),
        )]
    }

    /// The application's purge of `oid`, if it names an Audit Log, the one
    /// object type that purges.
    pub(super) fn purge(oid: ObjectIdentifier) -> Vec<Self> {
        if oid.object_type() != ObjectType::AUDIT_LOG {
            return Vec::new();
        }
        vec![Self {
            oid,
            change: Change::Purge,
        }]
    }
}

impl TargetValue {
    /// The value the write leaves, worked out under the guard as the handler
    /// works it out. `None` when a list edit cannot be worked out.
    fn resolve(
        &self,
        db: &ObjectDatabase,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Option<PropertyValue> {
        match self {
            Self::Written(value) => Some(crate::local_references::localize(
                db,
                oid,
                property,
                value.clone(),
            )),
            Self::ListEdit {
                service_data,
                remove,
            } => handlers::edited_list_value(db, service_data, *remove),
        }
    }
}

/// Stage one object's changes under the guard: `group` holds the request's
/// changes to that object, in request order. `None` when there is nothing
/// to stage: the object is gone or does not save, or its first written list
/// cannot be worked out. The writes go to the object together, so it can
/// fold them into one save.
fn stage_group(group: &[DurableTarget], db: &mut ObjectDatabase) -> Option<StageStep> {
    let first = group.first()?;
    let oid = first.oid;
    if matches!(first.change, Change::Purge) {
        return Some(db.get_mut(&oid)?.durable_writes_internal()?.stage_purge());
    }
    let mut writes = Vec::with_capacity(group.len());
    for target in group {
        let Change::Write {
            property,
            array_index,
            value,
        } = &target.change
        else {
            break;
        };
        // The request stops at a list it cannot edit; stage what comes first.
        let Some(value) = value.resolve(db, oid, *property) else {
            break;
        };
        writes.push(PendingWrite {
            property: *property,
            array_index: *array_index,
            value,
        });
    }
    if writes.is_empty() {
        return None;
    }
    Some(
        db.get_mut(&oid)?
            .durable_writes_internal()?
            .stage_writes(&writes),
    )
}

/// The objects a request staged writes on, each with the wait its stage
/// gave, which shows the object the stage is this request's.
#[must_use = "release staged writes in the critical section that makes them"]
pub(super) struct StagedWrites(Vec<(ObjectIdentifier, SaveWait)>);

impl StagedWrites {
    /// Release every staged write. Call it under the guard of the critical
    /// section that made the writes, after the writes. An object whose
    /// staged write was not taken saves the state it serves at once.
    pub(super) fn release(self, db: &mut ObjectDatabase) {
        for (oid, staged) in self.0 {
            if let Some(writes) = db
                .get_mut(&oid)
                .and_then(|object| object.durable_writes_internal())
            {
                writes.release_staged_write(&staged);
            }
        }
    }
}

/// Requests that found an object busy, per database (by address), so a test
/// can see a request reach its wait for another request's staged save.
#[cfg(test)]
static BUSY_WAITS: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

/// Note that a request on `db` found an object busy and is about to wait.
/// Only tests count it; otherwise this does nothing.
pub(super) fn note_busy(db: &Arc<RwLock<ObjectDatabase>>) {
    #[cfg(test)]
    {
        let key = Arc::as_ptr(db) as usize;
        let mut counts = BUSY_WAITS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match counts.iter_mut().find(|(at, _)| *at == key) {
            Some((_, count)) => *count += 1,
            None => counts.push((key, 1)),
        }
    }
    #[cfg(not(test))]
    let _ = db;
}

/// How many times requests on `db` have found an object busy.
#[cfg(test)]
pub(super) fn busy_waits(db: &Arc<RwLock<ObjectDatabase>>) -> usize {
    let key = Arc::as_ptr(db) as usize;
    BUSY_WAITS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(at, _)| *at == key)
        .map_or(0, |(_, count)| *count)
}

/// Wait for a staged save with the database guard dropped.
///
/// The save runs on the object's own writer thread, which Tokio cannot see.
/// Awaiting it directly would leave the runtime looking idle, and under a
/// paused test clock (`tokio::time::pause`) an idle runtime jumps virtual
/// time to the next timer: request timeouts would fire while the save was
/// still running. The wait therefore runs on the blocking pool, which keeps a
/// paused clock where it is until the save is done. A pool thread is held
/// only while a staged save is in flight, at most one per object.
pub(super) async fn saved(wait: SaveWait) {
    if wait.is_ready() {
        return;
    }
    let blocking = wait.clone();
    if tokio::task::spawn_blocking(move || blocking.block())
        .await
        .is_err()
    {
        wait.await;
    }
}

/// How long `stop()` waits for durable saves before it warns that storage
/// is holding it up.
pub(super) const SLOW_SAVE_WARNING: Duration = Duration::from_secs(5);
/// How often it warns again while it keeps waiting.
pub(super) const SLOW_SAVE_REPEAT: Duration = Duration::from_secs(30);

/// Settle every staged write in `db` once no request is left to take or
/// release one: `stop()` calls this after joining its requests (#1363).
/// Each object drops what it holds staged and puts storage back to the state
/// it serves, and every save the objects have queued is awaited off the
/// guard, so storage matches what the objects serve when `stop()` returns.
/// The wait has no limit: storage that stalls holds `stop()` up, which is
/// where the objects' drop would block otherwise. A warning names the
/// objects still saving after [`SLOW_SAVE_WARNING`], and again every
/// [`SLOW_SAVE_REPEAT`].
///
/// A second pass takes what the first waited for: an Audit Log batch whose
/// commit was still running, which the log takes once it has landed.
///
/// `stop()` does not join an application's `write_local`. One that staged
/// its save before the stop finds its stage dropped here when it comes back
/// for the guard, and saves in place under the guard instead; storage still
/// leads what the object serves.
///
/// An application holding the database is not waited for, as with the
/// Command runs `stop()` ends: the objects then settle from a task once it
/// lets go, and in any case put storage back when they are dropped.
pub(super) async fn settle_forgotten(db: &Arc<RwLock<ObjectDatabase>>) {
    for _ in 0..2 {
        let waits = match db.try_write() {
            Ok(mut db) => settle_all(&mut db),
            Err(_) => {
                settle_when_free(db);
                return;
            }
        };
        if waits.is_empty() {
            return;
        }
        wait_for_saves(db, &waits).await;
    }
}

/// Strict endpoint shutdown settlement after its request owners have joined.
/// Both passes wait asynchronously for the database, then release its guard
/// before waiting for queued saves. Completion describes finished attempts;
/// it does not report backend success or establish power-loss durability.
///
/// The endpoint retains one task running this future across stop cancellation.
/// Its database owner is installed before the future can be spawned or dropped.
pub fn settle_endpoint(
    db: Arc<RwLock<ObjectDatabase>>,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let owner = EndpointSettlementDatabase(Some(db));
    async move {
        let db = owner.0.as_ref().expect("settlement database");
        for _ in 0..2 {
            let waits = {
                let mut guard = db.write().await;
                settle_all(&mut guard)
            };
            if waits.is_empty() {
                return;
            }
            wait_for_saves(db, &waits).await;
        }
    }
}

/// Cancellation can make this task the final database holder, even before
/// its first poll. Preserve the endpoint's off-runtime destruction policy.
struct EndpointSettlementDatabase(Option<Arc<RwLock<ObjectDatabase>>>);

impl Drop for EndpointSettlementDatabase {
    fn drop(&mut self) {
        if let Some(db) = self.0.take() {
            drop(super::drop_database_off_runtime(db));
        }
    }
}

/// Settle `db`'s objects from a task once the application lets go of it.
/// Nothing waits for their saves; a runtime that shuts down first drops the
/// task, which is logged, and the objects then put storage back as they drop.
/// The task may outlive every other handle on the database, so it lets go of
/// its own off the runtime (#1513).
fn settle_when_free(db: &Arc<RwLock<ObjectDatabase>>) {
    tracing::info!("The database is held at stop(); staged writes settle once it is free");
    let db = Arc::clone(db);
    tokio::spawn(async move {
        let mut waiting = crate::command_lists::Waiting(Some(
            "runtime dropped the task settling staged writes after stop(); \
             each object puts storage back when it is dropped",
        ));
        settle_all(&mut *db.write().await);
        waiting.0 = None;
        drop(super::drop_database_off_runtime(db));
    });
}

/// Settle every object's forgotten staged writes: each object that keeps
/// state in storage, with the wait for the saves it still has to run.
fn settle_all(db: &mut ObjectDatabase) -> Vec<(ObjectIdentifier, SaveWait)> {
    let mut waits = Vec::new();
    db.for_each_object_mut(|oid, object| {
        if let Some(wait) = object
            .durable_writes_internal()
            .and_then(|writes| writes.settle_forgotten_writes())
            .filter(|wait| !wait.is_ready())
        {
            waits.push((oid, wait));
        }
    });
    waits
}

/// Wait for `waits` on the blocking pool, as [`saved`] waits for one, so a
/// paused clock stands still. While storage holds the wait up, warn which
/// objects are still saving.
async fn wait_for_saves(db: &Arc<RwLock<ObjectDatabase>>, waits: &[(ObjectIdentifier, SaveWait)]) {
    let blocking: Vec<SaveWait> = waits.iter().map(|(_, wait)| wait.clone()).collect();
    let mut all = tokio::task::spawn_blocking(move || blocking.iter().for_each(SaveWait::block));
    let started = tokio::time::Instant::now();
    let mut warn_at = started + SLOW_SAVE_WARNING;
    loop {
        match tokio::time::timeout_at(warn_at, &mut all).await {
            Ok(Ok(())) => return,
            Ok(Err(_)) => break,
            Err(_) => {
                let saving: Vec<String> = waits
                    .iter()
                    .filter(|(_, wait)| !wait.is_ready())
                    .map(|(oid, _)| oid.to_string())
                    .collect();
                tracing::warn!(
                    objects = %saving.join(", "),
                    waited_secs = started.elapsed().as_secs(),
                    "stop() is still waiting for storage to finish these objects' saves"
                );
                note_slow_saves(db);
                warn_at += SLOW_SAVE_REPEAT;
            }
        }
    }
    // The blocking pool is gone, as when the runtime shuts down: wait here.
    for (_, wait) in waits {
        wait.clone().await;
    }
}

/// Times `stop()` warned that storage held it up, per database (by
/// address), so a test can see the warning.
#[cfg(test)]
static SLOW_SAVE_WARNINGS: std::sync::Mutex<Vec<(usize, usize)>> =
    std::sync::Mutex::new(Vec::new());

/// Note a slow-save warning on `db`. Only tests count it; otherwise this
/// does nothing.
fn note_slow_saves(db: &Arc<RwLock<ObjectDatabase>>) {
    #[cfg(test)]
    {
        let key = Arc::as_ptr(db) as usize;
        let mut counts = SLOW_SAVE_WARNINGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match counts.iter_mut().find(|(at, _)| *at == key) {
            Some((_, count)) => *count += 1,
            None => counts.push((key, 1)),
        }
    }
    #[cfg(not(test))]
    let _ = db;
}

/// How many times `stop()` warned that storage held it up on `db`.
#[cfg(test)]
pub(super) fn slow_save_warnings(db: &Arc<RwLock<ObjectDatabase>>) -> usize {
    let key = Arc::as_ptr(db) as usize;
    SLOW_SAVE_WARNINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(at, _)| *at == key)
        .map_or(0, |(_, count)| *count)
}

/// Stage `targets` and wait for their saves without holding the database
/// guard. `targets` come in object order, so two requests never wait on each
/// other.
pub(super) async fn stage(
    db: &Arc<RwLock<ObjectDatabase>>,
    targets: Vec<DurableTarget>,
) -> StagedWrites {
    let mut staged = Vec::new();
    // Each object stages once, with all of the request's changes to it.
    for group in targets.chunk_by(|a, b| a.oid == b.oid) {
        loop {
            let step = {
                let mut guard = db.write().await;
                stage_group(group, &mut guard)
            };
            match step {
                Some(StageStep::Staged(wait)) => {
                    saved(wait.clone()).await;
                    staged.push((group[0].oid, wait));
                    break;
                }
                Some(StageStep::Busy(wait)) => {
                    note_busy(db);
                    let _ = tokio::time::timeout(BUSY_RECHECK, wait).await;
                }
                Some(StageStep::Skip) | None => break,
            }
        }
    }
    StagedWrites(staged)
}
