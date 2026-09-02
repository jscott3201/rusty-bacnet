use super::*;

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
    },
    /// The process the object represents, supplying its own `Present_Value`.
    Application { array_index: Option<u32> },
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
            },
            value,
            priority,
        )
        .await
    }

    /// Supply `Present_Value` for the process a local object represents, firing
    /// the notifications [`BACnetServer::write_local`] fires.
    ///
    /// Routes through [`BACnetObject::set_present_value_internal`], so an Input
    /// accepts the write while in service and refuses it while a client owns
    /// the value.
    ///
    /// [`BACnetObject::set_present_value_internal`]: bacnet_objects::traits::BACnetObject::set_present_value_internal
    pub async fn set_present_value_local(
        &self,
        oid: &ObjectIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::Application { array_index },
            value,
            priority,
        )
        .await
    }

    async fn write_local_as(
        &self,
        oid: &ObjectIdentifier,
        write: LocalWrite,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // Only a network write can carry OBJECT_NAME, so only it needs the name
        // index kept in step.
        let renaming = matches!(
            write,
            LocalWrite::Network { property, .. } if property == PropertyIdentifier::OBJECT_NAME
        );
        let life_safety = crate::life_safety_cov::is_life_safety_object(*oid);
        let exact_changes = {
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
                } => object.write_property(property, array_index, value, priority)?,
                LocalWrite::Application { array_index } => {
                    object.set_present_value_internal(array_index, value, priority)?
                }
            }
            if renaming {
                db.update_name_index(oid);
            }
            snapshots.changes(&db, std::slice::from_ref(oid))
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
        Ok(())
    }
}
