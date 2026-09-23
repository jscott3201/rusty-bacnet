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
        || write.property_identifier != PropertyIdentifier::DESCRIPTION
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
/// Handles `ReadProperty` and optionally authorized local Device.Description
/// `WriteProperty`, plus `Reject`/`Abort`. Full service parity is a later
/// packet. Inbound transactions reuse the
/// wire invoke ID directly and NEVER allocate from the shared outbound
/// client ID pool, so equal inbound/outbound numeric IDs stay unambiguous
/// via the ingress classifier + coordinator admission.
#[doc(hidden)]
pub struct EndpointResponder {
    db: Arc<RwLock<ObjectDatabase>>,
    egress: EndpointEgress,
    open: AtomicBool,
    device_writes: Option<(ObjectIdentifier, MutationAuthorizer)>,
}

impl EndpointResponder {
    #[doc(hidden)]
    pub fn new(db: Arc<RwLock<ObjectDatabase>>, egress: EndpointEgress) -> Self {
        Self {
            db,
            egress,
            open: AtomicBool::new(true),
            device_writes: None,
        }
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

    async fn write_device_property(
        &self,
        request: &ConfirmedRequestPdu,
        received: &ReceivedApdu,
    ) -> Result<(), Error> {
        let (device, authorizer) = self.device_writes.as_ref().expect("enabled Device writes");
        let write = WritePropertyRequest::decode(&request.service_request)?;
        {
            let mut db = self.db.write().await;
            if !self.open.load(Ordering::Acquire) {
                return Err(shutdown_error());
            }
            device_write_target(&mut db, *device, &write)?;
        }
        if write.property_array_index.is_some() {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
            });
        }
        let value = handlers::decode_write_property_value(
            write.property_identifier,
            None,
            &write.property_value,
        )?;
        if !matches!(
            value,
            PropertyValue::CharacterString(_) | PropertyValue::Null
        ) {
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
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
            target: MutationTarget::WriteProperty(write),
        };
        if !super::audit_notification::fail_closed_authorize(|| authorizer(&context)) {
            return Err(super::audit_notification::request_denied());
        }
        let mut db = self.db.write().await;
        // Close may win while this request waits for the database owner.
        if !self.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
        let MutationTarget::WriteProperty(write) = &context.target else {
            unreachable!("Device write context")
        };
        device_write_target(&mut db, *device, write)?.write_property(
            write.property_identifier,
            write.property_array_index,
            value,
            write.priority,
        )
    }

    /// Handles one inbound request, preserving provenance structurally.
    ///
    /// Preserves `link_layer_group` (raw), `is_group` (effective),
    /// `data_attributes`, `ingress_network` and `provenance` are threaded
    /// through without new policy decisions; `data_attributes` are forwarded
    /// on the reply send instead of being dropped.
    #[doc(hidden)]
    pub async fn handle(&self, mut received: ReceivedApdu) -> Result<bool, Error> {
        if !self.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
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

        let invoke_id = request.invoke_id;
        let mut response = if request.segmented {
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            })
        } else if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            confirmed_response::read_property_response(&self.db, &request).await
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

        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &response)?;
        if matches!(response, Apdu::ComplexAck(_))
            && encoded.len() > usize::from(request.max_apdu_length)
        {
            response = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            });
            encoded.clear();
            encode_apdu(&mut encoded, &response)?;
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
