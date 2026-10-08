use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::mutation::{
    MutationAuthorizationContext, MutationAuthorizer, MutationTarget, MutationTrust,
};
use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointEgress};
use bacnet_network::layer::ReceivedApdu;
use bacnet_services::write_property::WritePropertyRequest;

use super::confirmed_response;
use super::*;

#[allow(dead_code)]
fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

fn property_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

/// Check existence, scope and typed authority under the database write guard.
/// The callback runs between preflight and the identical commit-time check.
fn device_write_target<'a>(
    db: &'a mut ObjectDatabase,
    selected: ObjectIdentifier,
    write: &WritePropertyRequest,
) -> Result<bacnet_objects::device::DeviceAuthority<'a>, Error> {
    let object = db
        .get_mut(&write.object_identifier)
        .ok_or_else(|| property_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))?;
    if !object.property_list().contains(&write.property_identifier) {
        return Err(property_error(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ));
    }
    if write.object_identifier != selected
        || selected.object_type() != ObjectType::DEVICE
        || selected.instance_number() == ObjectIdentifier::MAX_INSTANCE
        || !matches!(
            write.property_identifier,
            PropertyIdentifier::DESCRIPTION | PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
        )
    {
        return Err(property_error(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ));
    }
    object
        .device_authority_internal()
        .filter(|device| device.object_identifier() == selected)
        .ok_or_else(|| property_error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
}

/// Composition-visible inbound responder (narrow service scope).
///
/// Handles `ReadProperty`, optionally authorized local Device
/// Description/active recipient `WriteProperty`, optionally `ReinitializeDevice`
/// through its handler, plus `Reject`/`Abort`. Full service parity is a later
/// packet. Inbound transactions reuse the
/// wire invoke ID directly and NEVER allocate from the shared outbound
/// client ID pool, so equal inbound/outbound numeric IDs stay unambiguous
/// via the ingress classifier + coordinator admission.
#[doc(hidden)]
pub struct EndpointResponder {
    /// `Some` until the responder drops; see [`Self::db`].
    db: Option<Arc<RwLock<ObjectDatabase>>>,
    egress: EndpointEgress,
    open: AtomicBool,
    device_writes: Option<(ObjectIdentifier, MutationAuthorizer)>,
    reinitialize: Option<(ReinitializeHandler, Option<String>)>,
    registered_port: Option<(ObjectIdentifier, std::sync::Weak<()>)>,
    read_work_limit: usize,
}

impl Drop for EndpointResponder {
    /// The responder's handle on the database goes through
    /// [`drop_database_off_runtime`](crate::server::drop_database_off_runtime):
    /// whichever role handle or dispatch task drops the responder last, in
    /// async code it never drops the objects on a runtime worker (#1561).
    fn drop(&mut self) {
        if let Some(db) = self.db.take() {
            drop(crate::server::drop_database_off_runtime(db));
        }
    }
}

impl EndpointResponder {
    #[doc(hidden)]
    pub fn new(db: Arc<RwLock<ObjectDatabase>>, egress: EndpointEgress) -> Self {
        Self {
            db: Some(db),
            egress,
            open: AtomicBool::new(true),
            device_writes: None,
            reinitialize: None,
            registered_port: None,
            read_work_limit: crate::server::ReadPropertyMultipleBudget::default()
                .max_result_elements,
        }
    }

    /// Result rows one ReadProperty may expand: its own row plus, for a
    /// Group's Present_Value, every member row (#1215). A read past it is
    /// aborted with OUT_OF_RESOURCES. The session validates it positive.
    #[doc(hidden)]
    pub fn with_read_work_limit(mut self, limit: usize) -> Self {
        self.read_work_limit = limit;
        self
    }

    /// The database, held until the responder drops.
    fn db(&self) -> &Arc<RwLock<ObjectDatabase>> {
        self.db.as_ref().expect("held until the responder drops")
    }

    /// Receiving-port identity selected by this owner, never inferred from DB rows.
    #[doc(hidden)]
    pub fn with_registered_port(
        mut self,
        oid: ObjectIdentifier,
        lease: std::sync::Weak<()>,
    ) -> Self {
        self.registered_port = Some((oid, lease));
        self
    }

    /// Install the Device authority validated by the session before startup.
    #[doc(hidden)]
    pub fn with_device_writes(
        mut self,
        device: ObjectIdentifier,
        authorizer: MutationAuthorizer,
    ) -> Self {
        self.device_writes = Some((device, authorizer));
        self
    }

    /// Install the ReinitializeDevice handler and the password a request must carry.
    /// The handler runs under the rules on [`ReinitializeHandler`].
    #[doc(hidden)]
    pub fn with_reinitialize(
        mut self,
        handler: ReinitializeHandler,
        password: Option<String>,
    ) -> Self {
        self.reinitialize = Some((handler, password));
        self
    }

    /// Refuses once the session owner has closed this responder.
    fn ensure_open(&self) -> Result<(), Error> {
        if self.open.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(shutdown_error())
        }
    }

    async fn write_device_property(
        &self,
        request: &ConfirmedRequestPdu,
        received: &ReceivedApdu,
    ) -> Result<(), Error> {
        let (device, authorizer) = self.device_writes.as_ref().expect("enabled Device writes");
        let write = WritePropertyRequest::decode(&request.service_request)
            .map_err(Error::into_request_reject)?;
        {
            let mut db = self.db().write().await;
            self.ensure_open()?;
            device_write_target(&mut db, *device, &write)?;
        }
        if write.property_array_index.is_some() {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
            });
        }
        // Description and Audit_Notification_Recipient, the only targets
        // `device_write_target` admits, are single values.
        let value = handlers::decode_write_property_value(
            write.property_identifier,
            None,
            false,
            &write.property_value,
        )?;
        if write.property_identifier == PropertyIdentifier::DESCRIPTION
            && !matches!(
                value,
                PropertyValue::CharacterString(_) | PropertyValue::Null
            )
        {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        let context = MutationAuthorizationContext {
            source_mac: received.source_mac.clone(),
            source_network: received.source_network.clone(),
            provenance: received.provenance,
            trust: MutationTrust::from_provenance(received.provenance),
            invoke_id: Some(request.invoke_id),
            service_choice: request.service_choice.into(),
            target: MutationTarget::WriteProperty(write),
        };
        if !super::audit_notification::fail_closed_authorize(|| authorizer(&context)) {
            return Err(super::audit_notification::request_denied());
        }
        let mut db = self.db().write().await;
        // Close may win while this request waits for the database owner.
        self.ensure_open()?;
        let MutationTarget::WriteProperty(write) = &context.target else {
            unreachable!("Device write context")
        };
        let mut authority = device_write_target(&mut db, *device, write)?;
        if write.property_identifier == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
            let source = bacnet_objects::device::AuditWriteSource {
                device: bacnet_types::constructed::BACnetRecipient::Address(
                    bacnet_types::constructed::BACnetAddress {
                        network_number: received
                            .source_network
                            .as_ref()
                            .map_or(0, |source| source.network),
                        mac_address: received.source_network.as_ref().map_or_else(
                            || received.source_mac.clone(),
                            |source| source.mac_address.clone(),
                        ),
                    },
                ),
                invoke_id: request.invoke_id,
            };
            authority.write_audit_recipient(
                write.property_array_index,
                value,
                write.priority,
                Some(&source),
            )
        } else {
            authority.write_property(
                write.property_identifier,
                write.property_array_index,
                value,
                write.priority,
            )
        }
    }

    /// Handles one inbound request, preserving provenance structurally.
    ///
    /// Direct provenance or any supplied capability selects checked original-socket
    /// egress before a prompt reply channel. Other ingress preserves ordinary
    /// addressing, prompt reply semantics and data attributes. Admitted service
    /// execution is independent of whether its saved response authority survives.
    #[doc(hidden)]
    pub async fn handle(&self, mut received: ReceivedApdu) -> Result<bool, Error> {
        self.ensure_open()?;
        let _registration_lease = match &self.registered_port {
            Some((_, lease)) => Some(lease.upgrade().ok_or_else(shutdown_error)?),
            None => None,
        };
        // Structural preservation: bind raw + effective group, attributes,
        // ingress identity and provenance so a future drop is compile-visible.
        let _link_layer_group = received.link_layer_group;
        let _ingress_network = received.ingress_network;
        let _provenance = received.provenance;
        let preserved_attributes = received.data_attributes.clone();
        if received.is_group {
            return Ok(false);
        }
        let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu.clone())? else {
            return Ok(false);
        };

        let response_route = received.response_route();
        let checked_response =
            received.provenance.is_direct_peer() || received.direct_response.is_some();
        let invoke_id = request.invoke_id;
        let reinitialization = self
            .reinitialize
            .as_ref()
            .filter(|_| request.service_choice == ConfirmedServiceChoice::REINITIALIZE_DEVICE);
        let mut response = if request.segmented {
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            })
        } else if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            confirmed_response::read_property_response(
                self.db(),
                &request,
                self.device_writes.is_some(),
                self.reinitialize.is_some(),
                self.registered_port.as_ref().map(|(oid, _)| *oid),
                self.read_work_limit,
            )
            .await
        } else if let Some((handler, password)) = reinitialization {
            let requester = reinitialize::Requester::new(
                &received.source_mac,
                received.source_network.as_ref(),
                received.provenance,
            );
            // Close may win while the request waits for the database owner;
            // it is refused then, before the handler runs.
            let still_open = || self.ensure_open();
            reinitialize::response(
                self.db(),
                &request,
                password,
                Some(handler),
                requester,
                still_open,
                |_| {}, // Endpoint sessions have no server DCC state.
            )
            .await
        } else if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY
            && self.device_writes.is_some()
        {
            match self.write_device_property(&request, &received).await {
                Ok(()) => Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: request.service_choice,
                }),
                Err(error) => confirmed_response::error_apdu_from_error(
                    invoke_id,
                    request.service_choice,
                    &error,
                ),
            }
        } else {
            Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            })
        };

        // Sizing uses the saved link limits after application execution. Invalid
        // authority is still rejected by checked send, never by address fallback.
        let max_apdu = response_route
            .max_apdu_length(request.max_apdu_length, received.source_network.as_ref())
            .unwrap_or(request.max_apdu_length);
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &response)?;
        if matches!(response, Apdu::ComplexAck(_)) && encoded.len() > usize::from(max_apdu) {
            response = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            });
            encoded.clear();
            encode_apdu(&mut encoded, &response)?;
        }

        if checked_response {
            drop(received.reply_tx.take());
            self.egress
                .admit_response_apdu(
                    encoded.to_vec(),
                    received.source_mac,
                    received.source_network,
                    response_route,
                )?
                .complete()
                .await
                .result?;
            return Ok(true);
        }

        if let Some(reply_tx) = received.reply_tx.take() {
            let apdu = encoded.freeze();
            let npdu = Npdu {
                is_network_message: false,
                expecting_reply: false,
                priority: NetworkPriority::NORMAL,
                destination: received.source_network,
                source: None,
                payload: apdu,
                ..Npdu::default()
            };
            let mut wrapped = BytesMut::new();
            encode_npdu(&mut wrapped, &npdu)?;
            let _ = reply_tx.send(wrapped.freeze());
            return Ok(true);
        }

        if let Some(source_network) = received.source_network {
            self.egress
                .send_apdu(
                    encoded.to_vec(),
                    EndpointApduDestination::Routed {
                        destination_network: source_network.network,
                        destination_mac: source_network.mac_address,
                        router_mac: received.source_mac,
                    },
                    false,
                    NetworkPriority::NORMAL,
                    preserved_attributes,
                )
                .await?;
            return Ok(true);
        }
        self.egress
            .send_apdu(
                encoded.to_vec(),
                EndpointApduDestination::Direct {
                    destination_mac: received.source_mac,
                },
                false,
                NetworkPriority::NORMAL,
                preserved_attributes,
            )
            .await?;
        Ok(true)
    }

    /// Internal close for the endpoint session owner only.
    ///
    /// Lifecycle control lives on the session owner; role handles expose no
    /// public lifecycle methods.
    #[doc(hidden)]
    pub fn close(&self) {
        self.open.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "endpoint_responder_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "endpoint_device_write_tests.rs"]
mod device_write_tests;

#[cfg(test)]
#[path = "endpoint_reinitialize_tests.rs"]
mod reinitialize_tests;
