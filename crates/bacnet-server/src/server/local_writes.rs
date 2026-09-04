use super::*;
use bacnet_objects::staging::StagingWritePlan;

#[cfg(test)]
#[path = "input_present_value_tests.rs"]
mod input_present_value_tests;

#[cfg(test)]
#[path = "staging_local_writes_tests.rs"]
mod staging_local_writes_tests;

/// What a local mutation is, and on whose behalf.
///
/// An Input gates `Present_Value` on `Out_Of_Service` in opposite directions
/// for the two, so this decides which check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalWrite {
    /// A client on the network, writing any property.
    Network {
        property: PropertyIdentifier,
        array_index: Option<u32>,
        priority: Option<u8>,
    },
    /// The application supplying a supported Input's logical `Present_Value`.
    ApplicationInputPresentValue,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Arm or rearm a Life Safety object from trusted local application logic.
    ///
    /// This uses the object-internal state channel under the database write
    /// lock. Network WriteProperty and WritePropertyMultiple remain unable to
    /// forge `Operation_Expected`. After the lock is released, an actual
    /// `Operation_Expected` readback change uses the exact Life Safety COV path;
    /// rearming to the current value emits no notification.
    pub async fn set_life_safety_operation_expected_local(
        &self,
        oid: &ObjectIdentifier,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        let changes = {
            let mut db = self.db.write().await;
            let snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::capture_oid(&db, *oid);
            let object = db.get_mut(oid).ok_or_else(|| Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            })?;
            object.set_life_safety_operation_expected_internal(operation)?;
            snapshots.changes(&db, std::slice::from_ref(oid))
        };
        for change in changes {
            Self::fire_life_safety_cov_notifications(
                &self.db,
                &self.network,
                &self.cov_table,
                &self.cov_in_flight,
                &self.notification_transactions,
                &self.comm_state,
                &self.config,
                &change.object_identifier,
                &change.changed_properties,
            )
            .await;
        }
        Ok(())
    }

    /// Write a property on a local object and fire the same post-write COV
    /// and event notifications that a network [`WriteProperty`] does.
    ///
    /// This is the server-owned local-mutation entry point: it performs the
    /// write under the database lock — routing `OBJECT_NAME` through the name
    /// uniqueness check and index refresh, exactly like the network handler —
    /// then releases the lock and runs the COV/event trigger path so a
    /// subscription observes a local mutation just as it would a network one.
    /// Low-level object setters deliberately bypass this notification owner.
    ///
    /// [`WriteProperty`]: bacnet_services::write_property::WritePropertyRequest
    pub async fn write_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::Network {
                property,
                array_index,
                priority,
            },
            value,
        )
        .await
    }

    /// Supply a supported Input's logical `Present_Value` from Rust application code.
    ///
    /// Analog Input, Binary Input, and Multi-state Input opt in. Binary values
    /// are logical INACTIVE/ACTIVE states after Polarity, not raw physical
    /// states. The update runs the existing post-write intrinsic-event and COV
    /// processing after the database lock is released; configured delays and
    /// distribution policy still determine when notifications are delivered.
    ///
    /// Applications are denied while `Out_Of_Service` is TRUE to protect a
    /// client's simulation value. That ownership rule is local policy, not a
    /// Standard mandate. Other object families fail closed; use
    /// [`BACnetServer::write_local`] for ordinary network-equivalent writes and
    /// commandable objects. Python exposure is tracked separately in #503.
    ///
    /// [`BACnetObject::set_present_value_internal`]: bacnet_objects::traits::BACnetObject::set_present_value_internal
    pub async fn set_present_value_local(
        &self,
        oid: &ObjectIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.write_local_as(oid, LocalWrite::ApplicationInputPresentValue, value)
            .await
    }

    async fn write_local_as(
        &self,
        oid: &ObjectIdentifier,
        write: LocalWrite,
        value: PropertyValue,
    ) -> Result<(), Error> {
        // Only a network write can carry OBJECT_NAME, so only it needs the name
        // index kept in step.
        let renaming = matches!(
            write,
            LocalWrite::Network { property, .. } if property == PropertyIdentifier::OBJECT_NAME
        );
        let life_safety = crate::life_safety_cov::is_life_safety_object(*oid);
        let (exact_changes, staging_plans) = {
            let mut db = self.db.write().await;
            let snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::capture_oid(&db, *oid);
            if db.get(oid).is_none() {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                });
            }
            if renaming {
                if let PropertyValue::CharacterString(ref new_name) = value {
                    db.check_name_available(oid, new_name)?;
                }
            }
            let object = db.get_mut(oid).expect("existence checked above");
            match write {
                LocalWrite::Network {
                    property,
                    array_index,
                    priority,
                } => object.write_property(property, array_index, value, priority)?,
                LocalWrite::ApplicationInputPresentValue => {
                    object.set_present_value_internal(value)?
                }
            }
            if renaming {
                db.update_name_index(oid);
            }
            let staging_plans = Self::take_staging_plans(&mut db, std::slice::from_ref(oid));
            (
                snapshots.changes(&db, std::slice::from_ref(oid)),
                staging_plans,
            )
        };

        Self::fire_event_notifications_with_bindings(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            oid,
            self.config.cov_retry_timeout_ms,
        )
        .await;
        if life_safety {
            for change in exact_changes {
                Self::fire_life_safety_cov_notifications(
                    &self.db,
                    &self.network,
                    &self.cov_table,
                    &self.cov_in_flight,
                    &self.notification_transactions,
                    &self.comm_state,
                    &self.config,
                    &change.object_identifier,
                    &change.changed_properties,
                )
                .await;
            }
        } else {
            Self::fire_cov_notifications(
                &self.db,
                &self.network,
                &self.cov_table,
                &self.cov_in_flight,
                &self.notification_transactions,
                &self.comm_state,
                &self.config,
                oid,
            )
            .await;
        }
        Self::execute_staging_plans(
            &self.db,
            &self.network,
            &self.cov_table,
            &self.cov_in_flight,
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            &self.comm_state,
            &self.config,
            staging_plans,
        )
        .await;
        Ok(())
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
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_staging_plans(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        plans: Vec<StagingWritePlan>,
    ) {
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
                        match database.get_mut(&target.object_identifier) {
                            Some(object) => match object.write_property(
                                PropertyIdentifier::PRESENT_VALUE,
                                None,
                                PropertyValue::Enumerated(u32::from(target.active)),
                                Some(plan.priority),
                            ) {
                                Ok(()) => TargetResult::Applied,
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
                            db,
                            network,
                            comm_state,
                            server_tsm,
                            notification_transactions,
                            device_bindings,
                            &target.object_identifier,
                            config.cov_retry_timeout_ms,
                        )
                        .await;
                        Self::fire_cov_notifications(
                            db,
                            network,
                            cov_table,
                            cov_in_flight,
                            notification_transactions,
                            comm_state,
                            config,
                            &target.object_identifier,
                        )
                        .await;
                    }
                }
            }

            let reliability_changed = {
                let mut database = db.write().await;
                database.get_mut(&plan.source).is_some_and(|source| {
                    source.complete_staging_write_plan_internal(plan.generation, all_succeeded)
                })
            };
            if reliability_changed {
                Self::fire_event_notifications_with_bindings(
                    db,
                    network,
                    comm_state,
                    server_tsm,
                    notification_transactions,
                    device_bindings,
                    &plan.source,
                    config.cov_retry_timeout_ms,
                )
                .await;
                Self::fire_cov_notifications(
                    db,
                    network,
                    cov_table,
                    cov_in_flight,
                    notification_transactions,
                    comm_state,
                    config,
                    &plan.source,
                )
                .await;
            }
        }
    }
}
