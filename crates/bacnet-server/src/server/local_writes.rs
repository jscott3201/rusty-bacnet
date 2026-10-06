use super::cov_notify_context::CovNotifyContext;
use super::*;
use crate::command_lists::TakenRuns;
use crate::handlers::{WriteCommitObserver, WriteTarget};
use bacnet_objects::staging::StagingWritePlan;
use bacnet_types::constructed::BACnetRecipient;

#[path = "local_write_finish.rs"]
mod finish;

#[path = "local_access_inputs.rs"]
mod access_inputs;
use finish::local_runtime;
pub(super) use finish::Committed;

#[cfg(test)]
#[path = "input_present_value_tests.rs"]
mod input_present_value_tests;

#[cfg(test)]
#[path = "staging_local_writes_tests.rs"]
mod staging_local_writes_tests;

#[cfg(test)]
#[path = "local_array_index_tests.rs"]
mod local_array_index_tests;

#[cfg(test)]
#[path = "local_index_staged_release_tests.rs"]
mod local_index_staged_release_tests;

/// What a local mutation is, and on whose behalf.
///
/// Inputs and noncommandable Values distinguish application updates from
/// network-equivalent writes, including their Out_Of_Service ownership checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LocalWrite<'s> {
    /// A trusted local program performing a network-equivalent property write.
    Property {
        property: PropertyIdentifier,
        array_index: Option<u32>,
        priority: Option<u8>,
    },
    /// The application supplying a supported object's logical `Present_Value`.
    ApplicationPresentValue,
    /// The application supplying a Loop's measured `Controlled_Variable_Value`.
    ApplicationControlledVariableValue,
    /// The application supplying one sample to an Averaging object.
    ApplicationAveragingSample,
    /// The application reporting an Averaging sample attempt that produced no
    /// value.
    ApplicationAveragingMiss,
    /// The application supplying a Life Safety object's `Tracking_Value`.
    ApplicationTrackingValue,
    /// The application reporting an access-control object's input: an
    /// access event, a credential read or a door's hardware state (#1132).
    ApplicationAccessInput(&'s bacnet_objects::access_control::AccessControlInput),
    /// One value of an inbound WriteGroup for a Channel's `Present_Value`, at
    /// the priority the request gave it (Clause 15.11). Under the write guard
    /// the Channel takes it only while its Control_Groups still holds `group`
    /// and its Channel_Number is still `number`; otherwise the write is
    /// skipped and queues nothing. With `inhibit_delay`, the run it queues
    /// loses its delays when Allow_Group_Delay_Inhibit is TRUE at that moment.
    /// It comes from the network: `requester`, the device that sent the
    /// WriteGroup, is the source its Audit record names, with no invoke ID
    /// (#1318).
    WriteGroup {
        group: u32,
        number: u16,
        priority: u8,
        inhibit_delay: bool,
        requester: &'s BACnetRecipient,
    },
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Arm or rearm a Life Safety object from trusted local application logic.
    ///
    /// This uses the object-internal state channel under the database write
    /// lock. Network WriteProperty and WritePropertyMultiple remain unable to
    /// forge `Operation_Expected`. After the lock is released, an actual
    /// `Operation_Expected` readback change uses the exact Life Safety COV path;
    /// rearming to the current value emits no notification.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything changes outside one. Once the
    /// object holds the operation, the timestamped capture and the COV
    /// fanout the change owes run as a task of their own in the server's
    /// request task set, which this future only waits for, so a caller
    /// dropped then skips none of it (#1520). `stop()` aborts that task with
    /// the other request tasks.
    pub async fn set_life_safety_operation_expected_local(
        &self,
        oid: &ObjectIdentifier,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        let runtime = local_runtime()?;
        self.active_network()?;
        // Owned, so the fanout can take it to a task of its own with no wait
        // between the change and the hand-off.
        let mut db = Arc::clone(&self.db).write_owned().await;
        let mut commit = crate::committed_cov::BackgroundCommit::new();
        commit.before_change(&db, *oid);
        let object = db.get_mut(oid).ok_or_else(|| Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;
        object.set_life_safety_operation_expected_internal(operation)?;
        // Only a Life Safety Point or Zone reports the change. Another object
        // whose setter takes the operation owes no COV, as before (#1520).
        if crate::life_safety_cov::is_life_safety_object(*oid) {
            commit.changed(*oid);
        }
        let fanout =
            super::cov_fanout::CovFanout::new(&self.local_cov_context(), &self.event_suppressions);
        self.finish_in_task(&runtime, async move {
            // Under the change's guard: the database, then the COV table.
            let committed = commit.finish(&fanout.db, &mut db, &fanout.cov_table).await;
            drop(db);
            fanout.fire(&committed).await;
        })
        .await
    }

    /// Write a property on a local object and fire the same post-write COV
    /// and event notifications that a network [`WriteProperty`] does. When target
    /// Audit reporting is configured, the same observer records eligible local
    /// writes with local Device provenance and no invoke ID; Device recipient
    /// changes retain their sole old/new pair owner. Eligible actual AV/BV Audit
    /// policy changes reserve immediate delivery before assignment; unavailable
    /// resources return SERVICES/SERVICE_REQUEST_DENIED without a policy change.
    ///
    /// This is the server-owned local-mutation entry point: it performs the
    /// write under the database lock — routing `OBJECT_NAME` through the name
    /// uniqueness check and index refresh, exactly like the network handler —
    /// then releases the lock and runs the COV/event trigger path so a
    /// subscription observes a local mutation just as it would a network one.
    /// `source` explicitly selects the Device or an existing initiating object.
    /// Tracked commands require a concrete selected Device; the selected Device
    /// retains correction ownership regardless of the local initiator. Unrelated
    /// properties preserve their behavior without a usable command origin.
    /// Low-level object setters deliberately bypass this notification owner.
    ///
    /// A NULL to a property that isn't commandable and has no NULL in its
    /// datatype succeeds and changes nothing, as over the network (#1396):
    /// Audit records it, and no COV, event or Schedule work follows it.
    ///
    /// An array index is checked first, as the WriteProperty handler checks
    /// it (#1426): UNKNOWN_PROPERTY for a property the object doesn't hold,
    /// PROPERTY_IS_NOT_AN_ARRAY for one that isn't an array. Neither the
    /// object nor Audit sees such a write. The writes a Command or Channel
    /// run makes here get the same check.
    ///
    /// # Cancellation
    ///
    /// Dropped before the write commits, the future makes no change. Once it
    /// has committed, the work the write owes (the event pass, the COV
    /// fanout, a re-evaluated Schedule's target fanout, a Staging plan, and
    /// the Command or Channel runs it started) runs as a task of its own in
    /// the server's request task set, which this future only waits for, so
    /// dropping the future skips none of it (#1367): subscribers hear of the
    /// change, and a run ends as it would have, reported to its property
    /// subscribers. `stop()` aborts that task with the other request tasks,
    /// as it does a request handler: what it hadn't sent by then is not
    /// sent, and a run it hadn't started ends as if none of its writes were
    /// made (#1324), In_Process FALSE with each command unsuccessful, or
    /// Write_Status FAILED. The call still returns `Ok(())` then: the write
    /// was made. A panic in that work is raised again in the caller, as it
    /// would be were the work done in place.
    ///
    /// The future must be awaited inside a Tokio runtime, which the task is
    /// spawned onto: outside one it fails with [`Error::Encoding`] before
    /// anything is written.
    ///
    /// [`WriteProperty`]: bacnet_services::write_property::WritePropertyRequest
    pub async fn write_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        source: crate::LocalCommandSource,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::Property {
                property,
                array_index,
                priority,
            },
            value,
            Some(source),
        )
        .await
    }

    /// Supply a supported object's logical `Present_Value` from Rust application code.
    ///
    /// Analog, Binary and Multi-state Inputs, noncommandable Values, Loop
    /// (the control algorithm's output) and Life Safety Point and Zone opt in.
    /// Binary Input values are logical INACTIVE/ACTIVE states after Polarity,
    /// not raw physical states. The update runs the existing post-write
    /// intrinsic-event and COV processing after the database lock is released;
    /// configured delays and distribution policy still control delivery, and a
    /// COV_Increment still filters Present_Value reports.
    ///
    /// Applications are denied while `Out_Of_Service` is TRUE to protect a
    /// client's simulation value. This is local policy for Inputs and required
    /// by the object clauses for the supported Values and Loop. NULL is an
    /// invalid application value. Other object families fail closed; use
    /// [`BACnetServer::write_local`] for network-equivalent writes and sourced
    /// commands on commandable objects. The Python binding exposes this as
    /// `BACnetServer.set_present_value_local`. A noncommandable Value that
    /// tracks its source (`set_value_source_tracking`, #1552) then publishes
    /// this server's Device as Value_Source, as Clause 19.5 asks of a write
    /// the local device makes.
    ///
    /// A Life Safety Point or Zone takes an Enumerated BACnetLifeSafetyState,
    /// a standard value or one from 256 to 65535, in or out of service, since
    /// clients never write its Present_Value. The update leaves
    /// Tracking_Value, Silenced and Operation_Expected alone, so any latching
    /// until reset is the application's (see
    /// [`BACnetServer::set_tracking_value_local`]). A change notifies
    /// SubscribeCOV and Present_Value property subscribers through the Life
    /// Safety COV path. The built-in objects run no intrinsic reporting, so
    /// the post-write event pass raises nothing for them.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    ///
    /// [`BACnetObject::set_present_value_internal`]: bacnet_objects::traits::BACnetObject::set_present_value_internal
    pub async fn set_present_value_local(
        &self,
        oid: &ObjectIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.write_local_as(oid, LocalWrite::ApplicationPresentValue, value, None)
            .await
    }

    /// Supply a Loop's measured `Controlled_Variable_Value` from the
    /// application that runs its control algorithm.
    ///
    /// The value is a finite REAL: another datatype fails with PROPERTY /
    /// INVALID_DATA_TYPE and NaN or an infinity with PROPERTY /
    /// VALUE_OUT_OF_RANGE. An unknown object fails with OBJECT /
    /// UNKNOWN_OBJECT and any object other than a Loop with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, the same errors as
    /// [`BACnetServer::set_present_value_local`]. Unlike Present_Value it is
    /// accepted while `Out_Of_Service` is TRUE, because Out_Of_Service
    /// decouples only the output and Reliability from the algorithm.
    ///
    /// The update runs the same post-write COV processing as
    /// `set_present_value_local` once the database lock is released. A
    /// SubscribeCOVProperty on `Controlled_Variable_Value` is notified of the
    /// change; a SubscribeCOV on the Loop is not, since the Loop's COV report
    /// carries this value without being triggered by it, so its next report
    /// carries the new value. The property stays read-only over the network.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    pub async fn set_controlled_variable_value_local(
        &self,
        oid: &ObjectIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::ApplicationControlledVariableValue,
            value,
            None,
        )
        .await
    }

    /// Record one sample for an Averaging object, taken by the application.
    ///
    /// The server samples an object that holds an `Object_Property_Reference`
    /// itself, every Window_Interval / Window_Samples seconds (#1144). For an
    /// object without one, the application samples whatever it averages,
    /// spacing its readings that far apart, and passes each result here. On an
    /// object the server samples, a call is one more attempt in the window
    /// and doesn't move the server's schedule. A value may be
    /// a BOOLEAN (FALSE and TRUE count as 0 and 1), Signed, Unsigned,
    /// Enumerated or finite REAL; the object keeps its statistics in REAL
    /// (Clause 12.5). `None` records an attempt that produced no value, such
    /// as a failed read: it counts toward Attempted_Samples but not
    /// Valid_Samples. Another datatype, Double included, fails with PROPERTY /
    /// INVALID_DATA_TYPE and NaN or an infinity with PROPERTY /
    /// VALUE_OUT_OF_RANGE, and a refused sample counts as neither attempted
    /// nor valid. An unknown object fails with OBJECT / UNKNOWN_OBJECT and any
    /// object other than an Averaging object with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as with
    /// [`BACnetServer::set_present_value_local`].
    ///
    /// Each call fills the next slot of the object's window, dropping the
    /// oldest once Window_Samples are held, and Minimum_Value, Maximum_Value,
    /// Average_Value, Attempted_Samples and Valid_Samples change together
    /// under the database lock; the server's COV processing runs once the
    /// lock is released. Averaging has no Table 13-1 row, so SubscribeCOV on
    /// it is refused, but SubscribeCOVProperty and
    /// SubscribeCOVPropertyMultiple are admitted: a numeric property is
    /// reported when it moves by the subscription's COV increment, or on any
    /// change when the subscription gives none (Table 13-1a). A move to or
    /// from the NaN or infinite value of an empty window is always reported,
    /// and staying at it never is. The report has no Status_Flags because the
    /// object has none. The Python binding exposes this as
    /// `BACnetServer.add_averaging_sample_local`.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    pub async fn add_averaging_sample_local(
        &self,
        oid: &ObjectIdentifier,
        sample: Option<PropertyValue>,
    ) -> Result<(), Error> {
        match sample {
            Some(value) => {
                self.write_local_as(oid, LocalWrite::ApplicationAveragingSample, value, None)
                    .await
            }
            None => {
                self.write_local_as(
                    oid,
                    LocalWrite::ApplicationAveragingMiss,
                    PropertyValue::Null,
                    None,
                )
                .await
            }
        }
    }

    /// Supply a Life Safety Point's or Zone's `Tracking_Value`, the live state
    /// the application derived, while the server holds the object.
    ///
    /// The value is an Enumerated naming a standard BACnetLifeSafetyState or
    /// one from the proprietary range 256..=65535. A reserved or larger number
    /// fails with PROPERTY / VALUE_OUT_OF_RANGE and another datatype with
    /// PROPERTY / INVALID_DATA_TYPE. An unknown object fails with OBJECT /
    /// UNKNOWN_OBJECT and any object other than a Life Safety Point or Zone
    /// with OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as with
    /// [`BACnetServer::set_present_value_local`]. Present_Value, Silenced and
    /// Operation_Expected are left as they are: the object derives nothing
    /// from Tracking_Value.
    ///
    /// While `Out_Of_Service` is TRUE the value replaces the one set aside for
    /// the return to service, and a client's simulated Tracking_Value keeps
    /// being served (#1108); the return to service then serves the latest
    /// application value and notifies its subscribers. In service, a change
    /// reaches SubscribeCOVProperty and SubscribeCOVPropertyMultiple
    /// subscribers of Tracking_Value through the Life Safety COV path once the
    /// database lock is released; whole-object SubscribeCOV reports don't
    /// carry it. The property stays read-only over the network in service.
    /// The Python binding exposes this as `BACnetServer.set_tracking_value_local`.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    pub async fn set_tracking_value_local(
        &self,
        oid: &ObjectIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.write_local_as(oid, LocalWrite::ApplicationTrackingValue, value, None)
            .await
    }

    /// Commit the write, then hand what it owes to a task of its own in the
    /// request task set and wait for that task (#1367). The commit holds the
    /// database guard to the hand-off with no wait in between, so a caller
    /// dropped once the write has committed leaves the task running: its
    /// event pass, COV fanout, Schedule fanout, Staging plan and runs all go
    /// ahead. `stop()` aborts the task with the other request tasks. With
    /// no Tokio runtime to run that task on, nothing is written.
    async fn write_local_as(
        &self,
        oid: &ObjectIdentifier,
        write: LocalWrite<'_>,
        value: PropertyValue,
        source: Option<crate::LocalCommandSource>,
    ) -> Result<(), Error> {
        let runtime = local_runtime()?;
        self.active_network()?;
        let Some(committed) = self
            .local_writer()
            .commit(oid, write, value, source)
            .await?
        else {
            return Ok(());
        };
        let runner = super::command_runs::CommandRunner::for_server(self);
        // A closed set (the server is stopping) drops the task, and with it
        // the guard and the runs, which then end unsuccessful (#1324).
        self.finish_in_task(&runtime, async move {
            let runs = runner.writer().finish(committed).await;
            if !runs.is_empty() {
                runner.start(runs);
            }
        })
        .await
    }

    /// Borrow the handles a local write uses, once the caller has checked
    /// that the network is still active.
    pub(super) fn local_writer(&self) -> LocalWriter<'_, T> {
        LocalWriter {
            db: &self.db,
            network: self
                .network
                .as_ref()
                .expect("running local mutation owns network"),
            cov_table: &self.cov_table,
            cov_in_flight: &self.cov_in_flight,
            notification_transactions: &self.notification_transactions,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            device_bindings: &self.device_bindings,
            event_suppressions: &self.event_suppressions,
            config: &self.config,
        }
    }

    /// Borrow the COV notification handles for a local mutation.
    fn local_cov_context(&self) -> CovNotifyContext<'_, T> {
        self.local_writer().cov_context()
    }

    /// Borrow the EventNotification handles for a local mutation.
    fn local_event_delivery(&self) -> EventDelivery<'_, T> {
        self.local_writer().event_delivery()
    }

    pub(super) async fn execute_initial_staging_plans(&self) {
        let staging_oids = {
            let database = self.db.read().await;
            database.find_by_type(ObjectType::STAGING)
        };
        let staging_plans = {
            let mut database = self.db.write().await;
            Self::take_staging_plans(&mut database, &staging_oids)
        };
        Self::execute_staging_plans(
            &self.local_event_delivery(),
            &self.local_cov_context(),
            staging_plans,
        )
        .await;
    }

    pub(super) fn take_staging_plans(
        db: &mut ObjectDatabase,
        oids: &[ObjectIdentifier],
    ) -> Vec<StagingWritePlan> {
        oids.iter()
            .filter_map(|oid| {
                db.get_mut(oid)
                    .and_then(|object| object.take_staging_write_plan_internal())
            })
            .collect()
    }

    /// Execute local Staging targets one mutation guard at a time.
    ///
    /// Every target mutation is generation-checked in the same database guard
    /// that applies it. Event/COV work runs only after that guard is released.
    pub(super) async fn execute_staging_plans(
        delivery: &EventDelivery<'_, T>,
        cov: &CovNotifyContext<'_, T>,
        plans: Vec<StagingWritePlan>,
    ) {
        let db = delivery.db;
        for plan in plans {
            let mut all_succeeded = true;
            for target in &plan.writes {
                enum TargetResult {
                    Applied,
                    Failed,
                    Stale,
                }

                let result = {
                    let mut database = db.write().await;
                    let current = database
                        .get(&plan.source)
                        .and_then(|source| source.staging_generation_internal())
                        == Some(plan.generation);
                    if !current {
                        TargetResult::Stale
                    } else {
                        let origin = crate::command_source::resolve_local(
                            &database,
                            crate::LocalCommandSource::Object(plan.source),
                        )
                        .ok();
                        match database.get_mut(&target.object_identifier) {
                            Some(object) => match crate::command_source::write_target(
                                object,
                                PropertyIdentifier::PRESENT_VALUE,
                                None,
                                PropertyValue::Enumerated(u32::from(target.active)),
                                Some(plan.priority),
                                origin.as_ref(),
                            ) {
                                Ok(()) => {
                                    // Timestamped references capture the
                                    // target change under this guard (#856).
                                    let capture = cov
                                        .cov_table
                                        .read()
                                        .await
                                        .timed_capture(target.object_identifier);
                                    capture.run(&database);
                                    TargetResult::Applied
                                }
                                Err(_) => TargetResult::Failed,
                            },
                            None => TargetResult::Failed,
                        }
                    }
                };

                match result {
                    TargetResult::Stale => break,
                    TargetResult::Failed => all_succeeded = false,
                    TargetResult::Applied => {
                        Self::fire_event_notifications_with_bindings(
                            delivery,
                            cov.cov_table,
                            &target.object_identifier,
                        )
                        .await;
                        Self::fire_cov_notifications(cov, &target.object_identifier).await;
                    }
                }
            }

            let reliability_changed = {
                let mut database = db.write().await;
                let changed = database.get_mut(&plan.source).is_some_and(|source| {
                    source.complete_staging_write_plan_internal(plan.generation, all_succeeded)
                });
                if changed {
                    let capture = cov.cov_table.read().await.timed_capture(plan.source);
                    capture.run(&database);
                }
                changed
            };
            if reliability_changed {
                Self::fire_event_notifications_with_bindings(delivery, cov.cov_table, &plan.source)
                    .await;
                Self::fire_cov_notifications(cov, &plan.source).await;
            }
        }
    }
}

/// The handles one local write borrows: the mutation under the database
/// guard, then the event, COV and Staging work it owes once the guard is
/// dropped. `write_local` writes through it, and so does each write of a
/// Command object's run (Clause 12.10).
pub(super) struct LocalWriter<'a, T: TransportPort + 'static> {
    pub(super) db: &'a Arc<RwLock<ObjectDatabase>>,
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) cov_table: &'a Arc<RwLock<CovSubscriptionTable>>,
    pub(super) cov_in_flight: &'a Arc<Semaphore>,
    pub(super) notification_transactions: &'a Arc<NotificationTransactions>,
    pub(super) comm_state: &'a Arc<CommState>,
    pub(super) learned_routers: &'a Arc<Mutex<LearnedRouterCache>>,
    pub(super) device_bindings: &'a Arc<RwLock<DeviceBindingTable>>,
    pub(super) event_suppressions: &'a Arc<super::event_suppression::EventSuppressions>,
    /// The server's config, which a run's task shares rather than copies
    /// (#1521).
    pub(super) config: &'a Arc<ServerConfig>,
}

impl<'a, T: TransportPort + 'static> LocalWriter<'a, T> {
    /// The COV notification view of these handles.
    pub(super) fn cov_context(&self) -> CovNotifyContext<'a, T> {
        CovNotifyContext {
            db: self.db,
            network: self.network,
            cov_table: self.cov_table,
            cov_in_flight: self.cov_in_flight,
            notification_transactions: self.notification_transactions,
            comm_state: self.comm_state,
            config: self.config,
        }
    }

    /// The EventNotification delivery view of these handles.
    pub(super) fn event_delivery(&self) -> EventDelivery<'a, T> {
        EventDelivery {
            db: self.db,
            network: self.network,
            comm_state: self.comm_state,
            learned_routers: self.learned_routers,
            notification_transactions: self.notification_transactions,
            device_bindings: self.device_bindings,
            suppressions: self.event_suppressions,
            retry_timeout_ms: self.config.cov_retry_timeout_ms,
            local_apdu_capacity: self.config.max_apdu_length,
        }
    }

    /// Make one local write and the post-write work it owes. Returns the
    /// Command and Channel runs it queued, directly or through a Schedule it
    /// changed, for the caller to start. They are taken under the write's
    /// guard and held in a [`TakenRuns`] until then, so dropping this future
    /// part way, after the commit, ends them rather than leaving their
    /// objects busy (#1324). `write_local` runs the two halves itself, the
    /// second as a task of its own (#1367).
    pub(super) async fn write(
        &self,
        oid: &ObjectIdentifier,
        write: LocalWrite<'_>,
        value: PropertyValue,
        source: Option<crate::LocalCommandSource>,
    ) -> Result<TakenRuns, Error> {
        match self.commit(oid, write, value, source).await? {
            Some(committed) => Ok(self.finish(committed).await),
            None => Ok(TakenRuns::default()),
        }
    }

    /// The first half of [`Self::write`]: the checks and the commit, under a
    /// database guard it keeps. `None` for a write that changed nothing and
    /// owes nothing more; otherwise the guard and what the write owes, for
    /// [`Self::finish`]. Nothing here waits once the object has the value,
    /// so a caller dropped before this returns made no change.
    pub(super) async fn commit(
        &self,
        oid: &ObjectIdentifier,
        write: LocalWrite<'_>,
        value: PropertyValue,
        source: Option<crate::LocalCommandSource>,
    ) -> Result<Option<Committed>, Error> {
        // Only a property write can carry OBJECT_NAME, so only it needs the name
        // index kept in step.
        let renaming = matches!(
            write,
            LocalWrite::Property { property, .. } if property == PropertyIdentifier::OBJECT_NAME
        );
        // A Pulse Converter's Input_Reference is judged as it commits (#1341).
        let input_reference = matches!(
            write,
            LocalWrite::Property { property, .. } if property == PropertyIdentifier::INPUT_REFERENCE
        );
        let life_safety = crate::life_safety_cov::is_life_safety_object(*oid);
        // A write the object saves first saves here, without the guard.
        let durable = match write {
            LocalWrite::Property {
                property,
                array_index,
                ..
            } => super::durable_writes::DurableTarget::local(*oid, property, array_index, &value),
            _ => Vec::new(),
        };
        let staged = super::durable_writes::stage(self.db, durable).await;
        {
            // Owned, so the rest of the write can take it to a task of its
            // own with no wait between the commit and the hand-off (#1367).
            let mut db = Arc::clone(self.db).write_owned().await;
            let snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::capture_oid(&db, *oid);
            if let Err(error) = precheck(&db, oid, write, &value) {
                // Nothing reached the object: what was staged goes back.
                staged.release(&mut db);
                return Err(error);
            }
            // A WriteGroup was planned under an earlier guard; the Channel
            // may have left the group or changed number since.
            let mut group_skips_delays = false;
            if let LocalWrite::WriteGroup {
                group,
                number,
                inhibit_delay,
                ..
            } = write
            {
                let object = db.get(oid).expect("existence checked above");
                if !super::write_group::qualifies(object, group, number) {
                    debug!(
                        channel = %oid,
                        group,
                        number,
                        "WriteGroup skips a Channel no longer in the group with that number"
                    );
                    return Ok(None);
                }
                group_skips_delays =
                    inhibit_delay && super::write_group::allows_delay_inhibit(object);
            }
            let mut audit = match write {
                LocalWrite::Property {
                    property,
                    array_index,
                    priority,
                } => {
                    // Device's recipient sink exclusively owns the old/new pair.
                    let recipient_sink = property
                        == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
                        && db
                            .get_mut(oid)
                            .and_then(|object| object.device_authority_internal())
                            .is_some();
                    if recipient_sink {
                        None
                    } else {
                        let mut audit = audit_reporter::WriteAudit::local(
                            self.config,
                            self.network,
                            self.notification_transactions,
                            &db,
                        );
                        let encoded = audit_reporter::small_value(&value).unwrap_or_default();
                        if let Some(audit) = &mut audit {
                            audit.before(
                                &db,
                                WriteTarget {
                                    oid: *oid,
                                    property,
                                    array_index,
                                    priority,
                                    value: &encoded,
                                },
                            );
                        }
                        audit
                    }
                }
                // Audited as the WriteProperty of the Channel's Present_Value
                // it amounts to, from the device that sent the WriteGroup.
                LocalWrite::WriteGroup {
                    priority,
                    requester,
                    ..
                } => {
                    let mut audit = audit_reporter::WriteAudit::requested_by(
                        self.config,
                        self.network,
                        self.notification_transactions,
                        requester.clone(),
                        None,
                    );
                    let encoded = audit_reporter::small_value(&value).unwrap_or_default();
                    audit.before(
                        &db,
                        WriteTarget {
                            oid: *oid,
                            property: PropertyIdentifier::PRESENT_VALUE,
                            array_index: None,
                            priority: Some(priority),
                            value: &encoded,
                        },
                    );
                    Some(audit)
                }
                LocalWrite::ApplicationPresentValue
                | LocalWrite::ApplicationControlledVariableValue
                | LocalWrite::ApplicationAveragingSample
                | LocalWrite::ApplicationAveragingMiss
                | LocalWrite::ApplicationTrackingValue
                | LocalWrite::ApplicationAccessInput(_) => None,
            };
            let value = match write {
                LocalWrite::Property { property, .. } => {
                    crate::local_references::localize(&db, *oid, property, value)
                }
                _ => value,
            };
            let null = crate::handlers::relinquish::is_null_value(&value);
            let prepared = match write {
                LocalWrite::Property {
                    property,
                    array_index,
                    priority,
                } => {
                    let encoded = audit_reporter::small_value(&value).unwrap_or_default();
                    audit.as_mut().and_then(|audit| {
                        audit.commit_policy(
                            &mut db,
                            WriteTarget {
                                oid: *oid,
                                property,
                                array_index,
                                priority,
                                value: &encoded,
                            },
                            &value,
                        )
                    })
                }
                LocalWrite::ApplicationPresentValue
                | LocalWrite::ApplicationControlledVariableValue
                | LocalWrite::ApplicationAveragingSample
                | LocalWrite::ApplicationAveragingMiss
                | LocalWrite::ApplicationTrackingValue
                | LocalWrite::ApplicationAccessInput(_)
                | LocalWrite::WriteGroup { .. } => None,
            };
            // The application's own Present_Value comes from this Device, which
            // a tracked noncommandable Value publishes as its source (#1552).
            let source = source.or(matches!(write, LocalWrite::ApplicationPresentValue)
                .then_some(crate::LocalCommandSource::ServerDevice));
            let command_origin =
                source.and_then(|source| crate::command_source::resolve_local(&db, source).ok());
            let result = prepared.unwrap_or_else(|| {
                let object = db.get_mut(oid).expect("existence checked above");
                match write {
                    LocalWrite::Property {
                        property,
                        array_index,
                        priority,
                    } => {
                        crate::device_view::check_executor_owned_write(*oid, property)?;
                        crate::command_source::write_target(
                            object,
                            property,
                            array_index,
                            value,
                            priority,
                            command_origin.as_ref(),
                        )
                    }
                    LocalWrite::ApplicationPresentValue => match &command_origin {
                        Some(origin) => object.set_present_value_from_internal(value, origin),
                        None => object.set_present_value_internal(value),
                    },
                    LocalWrite::ApplicationControlledVariableValue => {
                        object.set_controlled_variable_value_internal(value)
                    }
                    LocalWrite::ApplicationAveragingSample => {
                        object.add_averaging_sample_internal(Some(value))
                    }
                    LocalWrite::ApplicationAveragingMiss => {
                        object.add_averaging_sample_internal(None)
                    }
                    LocalWrite::ApplicationTrackingValue => {
                        object.set_tracking_value_internal(value)
                    }
                    LocalWrite::ApplicationAccessInput(input) => {
                        object.report_access_input_internal(input.clone())
                    }
                    LocalWrite::WriteGroup { priority, .. } => object.write_property(
                        PropertyIdentifier::PRESENT_VALUE,
                        None,
                        value,
                        Some(priority),
                    ),
                }
            });
            staged.release(&mut db);
            // A NULL the property left as it was succeeds unchanged
            // (`handlers::relinquish`): audited as the write it is, with no
            // post-write work, since nothing changed.
            if let (
                Err(error),
                LocalWrite::Property {
                    property,
                    array_index,
                    ..
                },
            ) = (&result, write)
            {
                let object = db.get(oid).expect("existence checked above");
                if null
                    && crate::handlers::relinquish::leaves_unchanged(
                        object,
                        property,
                        array_index,
                        error,
                    )
                {
                    if let Some(audit) = &mut audit {
                        audit.committed(&mut db);
                    }
                    return Ok(None);
                }
            }
            if let Err(error) = result {
                if let Some(audit) = &mut audit {
                    audit.failed(&mut db, &error);
                }
                return Err(error);
            }
            if let Some(audit) = &mut audit {
                audit.committed(&mut db);
            }
            if renaming {
                db.update_name_index(oid);
            }
            if input_reference {
                db.check_input_reference(oid);
            }
            let staging_plans =
                BACnetServer::<T>::take_staging_plans(&mut db, std::slice::from_ref(oid));
            let mut command_runs = TakenRuns::take(self.db, &mut db, std::slice::from_ref(oid));
            if group_skips_delays {
                super::write_group::skip_delays(&mut command_runs, *oid);
            }
            let changes = snapshots.changes(&db, std::slice::from_ref(oid));
            Ok(Some(Committed {
                db,
                oid: *oid,
                life_safety,
                changes,
                staging_plans,
                command_runs,
            }))
        }
    }
}

/// The checks made under the write's guard before the audit hooks or the
/// object see the value, in WriteProperty's order: the object exists, and
/// for a property write, the array index fits the property (WriteProperty's
/// own [`check_write_array_index`](crate::handlers::check_write_array_index))
/// and a new Object_Name is free. The name check calls
/// [`ObjectDatabase::check_name_available`] directly, which is what
/// WriteProperty's `check_and_prepare_name_write` runs for a CharacterString.
/// A Command's or Channel's write takes this path too; a Schedule's target
/// writes run the same index gate in `schedule::deliver`.
fn precheck(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    write: LocalWrite<'_>,
    value: &PropertyValue,
) -> Result<(), Error> {
    let object = db.get(oid).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;
    let LocalWrite::Property {
        property,
        array_index,
        ..
    } = write
    else {
        return Ok(());
    };
    crate::handlers::check_write_array_index(object, property, array_index)?;
    match value {
        PropertyValue::CharacterString(new_name) if property == PropertyIdentifier::OBJECT_NAME => {
            db.check_name_available(oid, new_name)
        }
        _ => Ok(()),
    }
}
