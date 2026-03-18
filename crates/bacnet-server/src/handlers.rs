//! Service handlers for incoming BACnet requests.
//!
//! Each handler function processes a decoded service request against an
//! ObjectDatabase and returns the encoded response bytes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, EventSummary, GetEventInformationAck, GetEventInformationRequest,
};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{
    EnableDisable, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

/// Property identifier for File Data (property 65 / 0x41).
const PROP_FILE_DATA: u32 = 0x0041;
use bytes::BytesMut;

use crate::cov::{CovSubscription, CovSubscriptionTable};

/// Handle a ReadProperty request.
///
/// Looks up the object and property in the database, encodes the value,
/// and returns the ReadPropertyACK service bytes.
pub fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data)?;

    let object = db.get(&request.object_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    let value = object.read_property(request.property_identifier, request.property_array_index)?;

    // Encode the PropertyValue as application-tagged bytes
    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, &value)?;

    let ack = ReadPropertyACK {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: value_buf.to_vec(),
    };

    ack.encode(buf);
    Ok(())
}

/// Handle a ReadPropertyMultiple request.
///
/// Iterates over each requested object+property pair. Per-property errors are
/// returned inline (as ReadResultElement with error) rather than failing the
/// entire request — this matches the BACnet spec (Clause 15.7).
pub fn handle_read_property_multiple(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyMultipleRequest::decode(service_data)?;

    let mut results = Vec::new();
    for spec in &request.list_of_read_access_specs {
        let mut elements = Vec::new();

        match db.get(&spec.object_identifier) {
            Some(object) => {
                for prop_ref in &spec.list_of_property_references {
                    // Expand ALL / REQUIRED / OPTIONAL per Clause 15.7.3.
                    let prop_ids: Vec<PropertyIdentifier> = match prop_ref.property_identifier {
                        PropertyIdentifier::ALL => object.property_list().to_vec(),
                        PropertyIdentifier::REQUIRED => object.required_properties().to_vec(),
                        PropertyIdentifier::OPTIONAL => {
                            let required: std::collections::HashSet<PropertyIdentifier> =
                                object.required_properties().iter().copied().collect();
                            object
                                .property_list()
                                .iter()
                                .copied()
                                .filter(|p| !required.contains(p))
                                .collect()
                        }
                        other => vec![other],
                    };

                    for prop_id in prop_ids {
                        let array_index = if prop_ref.property_identifier == prop_id {
                            prop_ref.property_array_index
                        } else {
                            None
                        };
                        match object.read_property(prop_id, array_index) {
                            Ok(value) => {
                                let mut value_buf = BytesMut::new();
                                match encode_property_value(&mut value_buf, &value) {
                                    Ok(()) => {
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: array_index,
                                            property_value: Some(value_buf.to_vec()),
                                            error: None,
                                        });
                                    }
                                    Err(_) => {
                                        // Encoding failure → per-property error
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: array_index,
                                            property_value: None,
                                            error: Some((ErrorClass::PROPERTY, ErrorCode::OTHER)),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let (err_class, err_code) = match &e {
                                    Error::Protocol { class, code } => (
                                        ErrorClass::from_raw(*class as u16),
                                        ErrorCode::from_raw(*code as u16),
                                    ),
                                    _ => (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
                                };
                                elements.push(ReadResultElement {
                                    property_identifier: prop_id,
                                    property_array_index: array_index,
                                    property_value: None,
                                    error: Some((err_class, err_code)),
                                });
                            }
                        }
                    }
                }
            }
            None => {
                for prop_ref in &spec.list_of_property_references {
                    elements.push(ReadResultElement {
                        property_identifier: prop_ref.property_identifier,
                        property_array_index: prop_ref.property_array_index,
                        property_value: None,
                        error: Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
                    });
                }
            }
        }

        results.push(ReadAccessResult {
            object_identifier: spec.object_identifier,
            list_of_results: elements,
        });
    }

    let ack = ReadPropertyMultipleACK {
        list_of_read_access_results: results,
    };
    ack.encode(buf);
    Ok(())
}

/// Handle a WritePropertyMultiple request.
///
/// Atomic per Clause 15.10: validates all properties first, then commits.
/// If any object or property fails validation, no writes are applied.
/// Returns the list of written object identifiers for COV/event notification.
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let request = WritePropertyMultipleRequest::decode(service_data)?;

    // Phase 1: Validate — decode all values and verify objects exist.
    #[allow(clippy::type_complexity)]
    let mut decoded_writes: Vec<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        PropertyValue,
        Option<u8>,
    )> = Vec::new();

    for spec in &request.list_of_write_access_specs {
        let oid = spec.object_identifier;
        if db.get(&oid).is_none() {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            });
        }
        for prop in &spec.list_of_properties {
            let (value, _) = bacnet_encoding::primitives::decode_application_value(&prop.value, 0)?;
            decoded_writes.push((
                oid,
                prop.property_identifier,
                prop.property_array_index,
                value,
                prop.priority,
            ));
        }
    }

    // Phase 2: Commit — apply all writes, rolling back on failure.
    let mut applied: Vec<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        PropertyValue,
    )> = Vec::new();

    for (oid, prop_id, array_index, value, priority) in &decoded_writes {
        let object = db.get_mut(oid).unwrap(); // validated in phase 1
                                               // Save old value for rollback (best-effort; read may fail for write-only props).
        let old_value = object.read_property(*prop_id, *array_index).ok();
        match object.write_property(*prop_id, *array_index, value.clone(), *priority) {
            Ok(()) => {
                if let Some(old) = old_value {
                    applied.push((*oid, *prop_id, *array_index, old));
                }
            }
            Err(e) => {
                // Rollback all previously applied writes.
                for (rb_oid, rb_prop, rb_idx, rb_val) in applied.into_iter().rev() {
                    if let Some(obj) = db.get_mut(&rb_oid) {
                        let _ = obj.write_property(rb_prop, rb_idx, rb_val, None);
                    }
                }
                return Err(e);
            }
        }
    }

    // Collect unique written OIDs.
    let mut written_oids = Vec::new();
    for (oid, _, _, _, _) in &decoded_writes {
        if !written_oids.contains(oid) {
            written_oids.push(*oid);
        }
    }

    Ok(written_oids)
}

/// Handle a WriteProperty request.
///
/// Looks up the object, decodes the property value, writes it, and returns
/// the written object identifier (the caller will send a SimpleACK and
/// may use the OID for COV/event notifications).
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest::decode(service_data)?;
    let oid = request.object_identifier;

    let object = db.get_mut(&oid).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    // Decode the application-tagged property value
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&request.property_value, 0)?;

    object.write_property(
        request.property_identifier,
        request.property_array_index,
        value,
        request.priority,
    )?;

    Ok(oid)
}

/// Handle a SubscribeCOV request.
///
/// If both optional fields are absent, this is a cancellation that removes an
/// existing subscription. Otherwise it creates or updates a subscription.
/// Returns an error if the monitored object does not exist in the database
/// (only checked for new subscriptions, not cancellations).
pub fn handle_subscribe_cov(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    let request = SubscribeCOVRequest::decode(service_data)?;

    if request.is_cancellation() {
        table.unsubscribe(
            source_mac,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
        );
        return Ok(());
    }

    // Verify the monitored object exists and supports COV (Clause 13.14.1.3.1)
    match db.get(&request.monitored_object_identifier) {
        None => {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            });
        }
        Some(obj) if !obj.supports_cov() => {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
            });
        }
        _ => {}
    }

    const MAX_COV_SUBSCRIPTIONS: usize = 1024;
    if table.len() >= MAX_COV_SUBSCRIPTIONS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        });
    }

    // Clause 13.14.1.1.4: "A value of zero shall indicate an indefinite
    // lifetime, without automatic cancellation."
    let expires_at = request.lifetime.and_then(|secs| {
        if secs == 0 {
            None // indefinite
        } else {
            Some(Instant::now() + Duration::from_secs(secs as u64))
        }
    });

    table.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(source_mac),
        subscriber_process_identifier: request.subscriber_process_identifier,
        monitored_object_identifier: request.monitored_object_identifier,
        issue_confirmed_notifications: request.issue_confirmed_notifications.unwrap_or(false),
        expires_at,
        last_notified_value: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
    });

    Ok(())
}

/// Handle a SubscribeCOVProperty request (Clause 13.14.2).
///
/// Like SubscribeCOV but subscribes to changes on a specific property.
pub fn handle_subscribe_cov_property(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    use bacnet_services::cov::SubscribeCOVPropertyRequest;

    let request = SubscribeCOVPropertyRequest::decode(service_data)?;

    if request.is_cancellation() {
        table.unsubscribe(
            source_mac,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
        );
        return Ok(());
    }

    // Verify the monitored object exists
    let object = db
        .get(&request.monitored_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    // Verify the monitored property exists on this object
    object
        .read_property(
            request.monitored_property_identifier,
            request.monitored_property_array_index,
        )
        .map_err(|_| Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
        })?;

    const MAX_COV_SUBSCRIPTIONS: usize = 1024;
    if table.len() >= MAX_COV_SUBSCRIPTIONS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        });
    }

    // Clause 13.14.1.1.4: lifetime=0 means indefinite
    let expires_at = request.lifetime.and_then(|secs| {
        if secs == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(secs as u64))
        }
    });

    table.subscribe(CovSubscription {
        subscriber_mac: MacAddr::from_slice(source_mac),
        subscriber_process_identifier: request.subscriber_process_identifier,
        monitored_object_identifier: request.monitored_object_identifier,
        issue_confirmed_notifications: request.issue_confirmed_notifications.unwrap_or(false),
        expires_at,
        last_notified_value: None,
        monitored_property: Some(request.monitored_property_identifier),
        monitored_property_array_index: request.monitored_property_array_index,
        cov_increment: request.cov_increment,
    });

    Ok(())
}

/// Handle a WhoHas request and return an IHave response if we have the object.
///
/// Returns `Some(IHaveRequest)` if we have the requested object, `None` otherwise.
pub fn handle_who_has(
    db: &ObjectDatabase,
    service_data: &[u8],
    device_oid: ObjectIdentifier,
) -> Result<Option<IHaveRequest>, Error> {
    let request = WhoHasRequest::decode(service_data)?;

    // Check device instance range
    let instance = device_oid.instance_number();
    if let (Some(low), Some(high)) = (request.low_limit, request.high_limit) {
        if instance < low || instance > high {
            return Ok(None);
        }
    }

    // Search for the object
    match &request.object {
        WhoHasObject::Identifier(oid) => {
            if let Some(obj) = db.get(oid) {
                return Ok(Some(IHaveRequest {
                    device_identifier: device_oid,
                    object_identifier: *oid,
                    object_name: obj.object_name().to_string(),
                }));
            }
        }
        WhoHasObject::Name(name) => {
            for (oid, obj) in db.iter_objects() {
                if obj.object_name() == name {
                    return Ok(Some(IHaveRequest {
                        device_identifier: device_oid,
                        object_identifier: oid,
                        object_name: name.clone(),
                    }));
                }
            }
        }
    }

    Ok(None)
}

/// Handle a CreateObject request.
///
/// Supports creating objects by type (server picks instance) or by identifier.
/// Returns the encoded ObjectIdentifier of the created object (ComplexAck payload).
pub fn handle_create_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = CreateObjectRequest::decode(service_data)?;

    const MAX_OBJECTS: usize = 10_000;
    if db.len() >= MAX_OBJECTS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_FOR_OBJECT.to_raw() as u32,
        });
    }

    let (object_type, instance) = match &request.object_specifier {
        ObjectSpecifier::Type(obj_type) => {
            // Find next available instance number (O(n) via HashSet lookup)
            let existing: HashSet<u32> = db
                .find_by_type(*obj_type)
                .iter()
                .map(|oid| oid.instance_number())
                .collect();
            let next = (1u32..=4_194_303)
                .find(|i| !existing.contains(i))
                .ok_or_else(|| Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_FOR_OBJECT.to_raw() as u32,
                })?;
            (*obj_type, next)
        }
        ObjectSpecifier::Identifier(oid) => {
            if db.get(oid).is_some() {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS.to_raw() as u32,
                });
            }
            (oid.object_type(), oid.instance_number())
        }
    };

    let name = format!("{:?}-{}", object_type, instance);

    // Create the appropriate object based on type.
    // Analog objects use engineering units 95 (no-units) as default.
    // Multistate objects use 2 states as default.
    let object: Box<dyn bacnet_objects::traits::BACnetObject> =
        if object_type == ObjectType::ANALOG_INPUT {
            Box::new(bacnet_objects::analog::AnalogInputObject::new(
                instance, &name, 95,
            )?)
        } else if object_type == ObjectType::ANALOG_OUTPUT {
            Box::new(bacnet_objects::analog::AnalogOutputObject::new(
                instance, &name, 95,
            )?)
        } else if object_type == ObjectType::BINARY_INPUT {
            Box::new(bacnet_objects::binary::BinaryInputObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::BINARY_OUTPUT {
            Box::new(bacnet_objects::binary::BinaryOutputObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::BINARY_VALUE {
            Box::new(bacnet_objects::binary::BinaryValueObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_INPUT {
            Box::new(bacnet_objects::multistate::MultiStateInputObject::new(
                instance, &name, 2,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_OUTPUT {
            Box::new(bacnet_objects::multistate::MultiStateOutputObject::new(
                instance, &name, 2,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_VALUE {
            Box::new(bacnet_objects::multistate::MultiStateValueObject::new(
                instance, &name, 2,
            )?)
        } else {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
            });
        };

    let created_oid = object.object_identifier();
    db.add(object)?;

    // Apply list_of_initial_values (Clause 15.3.1.1).
    // On failure, remove the created object and return the error.
    for pv in &request.list_of_initial_values {
        let (value, _) = match bacnet_encoding::primitives::decode_application_value(&pv.value, 0) {
            Ok(v) => v,
            Err(e) => {
                db.remove(&created_oid);
                return Err(e);
            }
        };
        if let Some(obj) = db.get_mut(&created_oid) {
            if let Err(e) = obj.write_property(
                pv.property_identifier,
                pv.property_array_index,
                value,
                pv.priority,
            ) {
                db.remove(&created_oid);
                return Err(e);
            }
        }
    }

    // Encode the created object identifier as the ACK
    bacnet_encoding::primitives::encode_app_object_id(buf, &created_oid);
    Ok(())
}

/// Handle a DeleteObject request.
///
/// Removes the object from the database. Returns an error if the object
/// doesn't exist or is the Device object (which cannot be deleted).
pub fn handle_delete_object(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    let request = DeleteObjectRequest::decode(service_data)?;

    // Cannot delete the Device object
    if request.object_identifier.object_type() == ObjectType::DEVICE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw() as u32,
        });
    }

    db.remove(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    Ok(())
}

/// Validate a request password against the configured password.
///
/// Returns `Ok(())` if no password is configured, or if the request password matches.
/// Returns `Err(Error::Protocol { SECURITY, PASSWORD_FAILURE })` on mismatch or missing password.
/// Uses constant-time comparison to prevent timing side-channel attacks.
fn validate_password(
    configured: &Option<String>,
    request_pw: &Option<String>,
) -> Result<(), Error> {
    if let Some(ref expected) = configured {
        match request_pw {
            Some(ref pw) if constant_time_eq(pw.as_bytes(), expected.as_bytes()) => Ok(()),
            _ => Err(Error::Protocol {
                class: ErrorClass::SECURITY.to_raw() as u32,
                code: ErrorCode::PASSWORD_FAILURE.to_raw() as u32,
            }),
        }
    } else {
        Ok(())
    }
}

/// Constant-time byte-slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len = a.len().max(b.len());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..len {
        let x = if i < a.len() { a[i] } else { 0 };
        let y = if i < b.len() { b[i] } else { 0 };
        diff |= x ^ y;
    }
    diff == 0
}

/// Handle a DeviceCommunicationControl request.
///
/// Decodes the request, stores the new communication state into the shared
/// `AtomicU8` (0 = Enable, 1 = Disable, 2 = DisableInitiation), and returns
/// the requested state plus optional duration (minutes). Per Clause 16.4.3,
/// the caller should auto-revert to ENABLE after the duration expires.
pub fn handle_device_communication_control(
    service_data: &[u8],
    comm_state: &AtomicU8,
    dcc_password: &Option<String>,
) -> Result<(EnableDisable, Option<u16>), Error> {
    let request = DeviceCommunicationControlRequest::decode(service_data)?;
    validate_password(dcc_password, &request.password)?;
    // Clause 16.1.1.3.1: deprecated DISABLE (value 1) shall be rejected
    if request.enable_disable == EnableDisable::DISABLE {
        return Err(Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
        });
    }
    let new_state = if request.enable_disable == EnableDisable::ENABLE {
        0u8
    } else if request.enable_disable == EnableDisable::DISABLE_INITIATION {
        2u8
    } else {
        return Err(Error::Encoding("unknown EnableDisable value".into()));
    };
    comm_state.store(new_state, Ordering::Release);
    tracing::debug!(
        "DeviceCommunicationControl: state set to {:?} ({}), duration={:?} min",
        request.enable_disable,
        new_state,
        request.time_duration
    );
    Ok((request.enable_disable, request.time_duration))
}

/// Handle a ReinitializeDevice request.
///
/// Returns the requested state. The caller decides what action to take.
pub fn handle_reinitialize_device(
    service_data: &[u8],
    reinit_password: &Option<String>,
) -> Result<(), Error> {
    let request = ReinitializeDeviceRequest::decode(service_data)?;
    validate_password(reinit_password, &request.password)?;
    // Accept the request (SimpleAck). Actual reinitialization is left
    // to the application layer.
    Ok(())
}

/// Handle a GetEventInformation request.
///
/// Returns event summaries for objects whose event_state is not NORMAL.
/// Supports pagination via `last_received_object_identifier` (Clause 13.9).
pub fn handle_get_event_information(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    const MAX_SUMMARIES: usize = 25;

    let request = GetEventInformationRequest::decode(service_data)?;

    let mut summaries = Vec::new();
    let mut skipping = request.last_received_object_identifier.is_some();
    let mut more_events = false;

    for (oid, object) in db.iter_objects() {
        // Skip objects up to and including last_received_object_identifier.
        if skipping {
            if Some(oid) == request.last_received_object_identifier {
                skipping = false;
            }
            continue;
        }

        if let Ok(PropertyValue::Enumerated(state)) =
            object.read_property(PropertyIdentifier::EVENT_STATE, None)
        {
            if state != bacnet_types::enums::EventState::NORMAL.to_raw() {
                if summaries.len() >= MAX_SUMMARIES {
                    more_events = true;
                    break;
                }

                let notification_class = object
                    .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Unsigned(n) => Some(n as u32),
                        _ => None,
                    })
                    .unwrap_or(0);

                let event_enable = object
                    .read_property(PropertyIdentifier::EVENT_ENABLE, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => data.first().map(|b| b >> 5),
                        _ => None,
                    })
                    .unwrap_or(0x07);

                let notify_type = object
                    .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::Enumerated(n) => Some(n),
                        _ => None,
                    })
                    .unwrap_or(0);

                let event_priorities =
                    ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, notification_class)
                        .ok()
                        .and_then(|nc_oid| db.get(&nc_oid))
                        .and_then(|nc_obj| {
                            nc_obj
                                .read_property(PropertyIdentifier::PRIORITY, None)
                                .ok()
                        })
                        .and_then(|v| match v {
                            PropertyValue::OctetString(bytes) if bytes.len() == 3 => {
                                Some([bytes[0] as u32, bytes[1] as u32, bytes[2] as u32])
                            }
                            _ => None,
                        })
                        .unwrap_or([0, 0, 0]);

                let acked = object
                    .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::BitString { data, .. } => data.first().map(|b| b >> 5),
                        _ => None,
                    })
                    .unwrap_or(0x07);

                // Try to read EVENT_TIME_STAMPS from the object if available
                let event_timestamps = object
                    .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                    .ok()
                    .and_then(|v| match v {
                        PropertyValue::List(items) if items.len() == 3 => {
                            // Each item should be a timestamp — extract sequence numbers
                            // For now, fall back to defaults since full timestamp parsing
                            // requires constructed type support
                            None
                        }
                        _ => None,
                    })
                    .unwrap_or([
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                    ]);

                summaries.push(EventSummary {
                    object_identifier: oid,
                    event_state: state,
                    acknowledged_transitions: acked,
                    event_timestamps,
                    notify_type,
                    event_enable,
                    event_priorities,
                    notification_class,
                });
            }
        }
    }

    let ack = GetEventInformationAck {
        list_of_event_summaries: summaries,
        more_events,
    };

    ack.encode(buf);
    Ok(())
}

/// Handle an AcknowledgeAlarm request (Clause 13.3).
///
/// Decodes the request, verifies the referenced object exists in the database,
/// updates the acknowledged_transitions bitfield, and returns Ok(()) to indicate
/// a SimpleACK response.
pub fn handle_acknowledge_alarm(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    let request = AcknowledgeAlarmRequest::decode(service_data)?;

    // Map event_state_acknowledged → transition bit (Clause 13.3.2).
    let transition_bit: u8 = match EventState::from_raw(request.event_state_acknowledged) {
        s if s == EventState::NORMAL => 0x04, // TO_NORMAL
        s if s == EventState::FAULT => 0x02,  // TO_FAULT
        _ => 0x01,                            // TO_OFFNORMAL
    };

    let object = db
        .get_mut(&request.event_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    object.acknowledge_alarm(transition_bit)?;

    Ok(())
}

/// Handle a SubscribeCOVPropertyMultiple request (Clause 13.16).
///
/// Creates individual COV subscriptions for each property in each object
/// referenced by the request.
pub fn handle_subscribe_cov_property_multiple(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let request = SubscribeCOVPropertyMultipleRequest::decode(service_data)?;

    let confirmed = request.issue_confirmed_notifications.unwrap_or(false);

    for spec in &request.list_of_cov_subscription_specifications {
        // Verify object exists and supports COV
        match db.get(&spec.monitored_object_identifier) {
            None => {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                });
            }
            Some(obj) if !obj.supports_cov() => {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
                });
            }
            _ => {}
        }

        // Create a subscription for each property reference
        for cov_ref in &spec.list_of_cov_references {
            table.subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(source_mac),
                subscriber_process_identifier: request.subscriber_process_identifier,
                monitored_object_identifier: spec.monitored_object_identifier,
                issue_confirmed_notifications: confirmed,
                expires_at: None, // SubscribeCOVPropertyMultiple has no lifetime
                last_notified_value: None,
                monitored_property: Some(cov_ref.monitored_property.property_identifier),
                monitored_property_array_index: cov_ref.monitored_property.property_array_index,
                cov_increment: cov_ref.cov_increment,
            });
        }
    }

    Ok(())
}

/// Handle a WriteGroup request (Clause 15.11).
///
/// WriteGroup is an unconfirmed service that writes values to Channel objects.
/// Decodes the request and returns the parsed data for the server to apply.
pub fn handle_write_group(
    service_data: &[u8],
) -> Result<bacnet_services::write_group::WriteGroupRequest, Error> {
    bacnet_services::write_group::WriteGroupRequest::decode(service_data)
}

/// Handle a GetEnrollmentSummary request (Clause 13.11).
///
/// Decodes filtering parameters and iterates event-enrollment objects in the
/// database, returning those that match the filter criteria.
pub fn handle_get_enrollment_summary(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::enrollment_summary::{
        EnrollmentSummaryEntry, GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };

    let request = GetEnrollmentSummaryRequest::decode(service_data)?;

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let oid = object.object_identifier();

        // Read event state
        let event_state = object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Enumerated(e) => Some(e),
                _ => None,
            })
            .unwrap_or(0);

        // Skip NORMAL objects unless the filter specifically asks for them
        if let Some(filter_state) = request.event_state_filter {
            if event_state != filter_state.to_raw() {
                continue;
            }
        }

        // Read notification class
        let notification_class = object
            .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Unsigned(n) => Some(n as u16),
                _ => None,
            })
            .unwrap_or(0);

        // Apply notification class filter
        if let Some(nc_filter) = request.notification_class_filter {
            if notification_class != nc_filter {
                continue;
            }
        }

        // Apply priority filter
        if let Some(ref pf) = request.priority_filter {
            // Use notification class priority (simplified — use 0 as default)
            let priority = 0u8;
            if priority < pf.min_priority || priority > pf.max_priority {
                continue;
            }
        }

        // Only include objects with event detection support
        if event_state == 0 && request.event_state_filter.is_none() {
            continue; // Skip NORMAL unless explicitly requested
        }

        entries.push(EnrollmentSummaryEntry {
            object_identifier: oid,
            event_type: bacnet_types::enums::EventType::CHANGE_OF_STATE,
            event_state: bacnet_types::enums::EventState::from_raw(event_state),
            priority: 0,
            notification_class,
        });
    }

    let ack = GetEnrollmentSummaryAck { entries };
    ack.encode(buf);
    Ok(())
}

/// Handle a ConfirmedTextMessage request (Clause 16.5).
///
/// Decodes and validates the request. Returns Ok(request) so the server
/// can deliver the message to the application layer.
pub fn handle_text_message(
    service_data: &[u8],
) -> Result<bacnet_services::text_message::TextMessageRequest, Error> {
    bacnet_services::text_message::TextMessageRequest::decode(service_data)
}

/// Handle a LifeSafetyOperation request (Clause 13.13).
///
/// Decodes the request and returns Ok(()) for SimpleACK. The actual
/// operation should be applied by the server dispatch to the appropriate
/// life safety objects.
pub fn handle_life_safety_operation(service_data: &[u8]) -> Result<(), Error> {
    let _request = bacnet_services::life_safety::LifeSafetyOperationRequest::decode(service_data)?;
    Ok(())
}

/// Handle a GetAlarmSummary request (Clause 13.10).
///
/// No request parameters. Iterates all objects in the database and returns
/// those with event_state != NORMAL.
pub fn handle_get_alarm_summary(db: &ObjectDatabase, buf: &mut BytesMut) -> Result<(), Error> {
    use bacnet_services::alarm_summary::{AlarmSummaryEntry, GetAlarmSummaryAck};

    let mut entries = Vec::new();
    for (_oid, object) in db.iter_objects() {
        let oid = object.object_identifier();
        let event_state = object
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Enumerated(e) => Some(e),
                _ => None,
            })
            .unwrap_or(0);

        if event_state != 0 {
            // NORMAL = 0, any other value is an alarm state
            let acked = object
                .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::BitString {
                        unused_bits, data, ..
                    } => Some((unused_bits, data)),
                    _ => None,
                })
                .unwrap_or((5, vec![0xE0])); // all acknowledged by default

            entries.push(AlarmSummaryEntry {
                object_identifier: oid,
                alarm_state: bacnet_types::enums::EventState::from_raw(event_state),
                acknowledged_transitions: acked,
            });
        }
    }

    let ack = GetAlarmSummaryAck { entries };
    ack.encode(buf);
    Ok(())
}

/// Handle a ReadRange request (Clause 15.8).
///
/// Reads items from a list property (e.g., LOG_BUFFER) with optional range
/// filtering by position, sequence number, or time.
pub fn handle_read_range(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};

    let request = ReadRangeRequest::decode(service_data)?;

    let object = db.get(&request.object_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    // Read the full property value (list)
    let value = object.read_property(request.property_identifier, request.property_array_index)?;

    let items = match value {
        PropertyValue::List(items) => items,
        _ => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_A_LIST.to_raw() as u32,
            });
        }
    };

    let total = items.len();

    // Apply range filtering
    let (selected, first_item, last_item) = match &request.range {
        None => {
            // No range: return all items
            (items, true, true)
        }
        Some(RangeSpec::ByPosition {
            reference_index,
            count,
        }) => {
            let ref_idx = *reference_index as usize;
            let cnt = *count;
            // Clause 15.8.1.1.4.1.1: If the index does not exist, no items match.
            if cnt == 0 || total == 0 || ref_idx == 0 || ref_idx > total {
                (Vec::new(), true, true)
            } else if cnt > 0 {
                let start = ref_idx - 1; // 1-based to 0-based
                let end = (start + cnt as usize).min(total);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            } else {
                let abs_count = cnt.unsigned_abs() as usize;
                let end = ref_idx; // ref_idx is 1-based, used as exclusive end in 0-based
                let start = end.saturating_sub(abs_count);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            }
        }
        Some(RangeSpec::BySequenceNumber {
            reference_seq,
            count,
        }) => {
            // Treat sequence numbers as 1-based indices into the list.
            let ref_idx = *reference_seq as usize;
            let cnt = *count;
            if cnt == 0 || total == 0 {
                (Vec::new(), true, true)
            } else if cnt > 0 {
                let start = ref_idx.min(total).saturating_sub(1);
                let end = (start + cnt as usize).min(total);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            } else {
                let abs_count = cnt.unsigned_abs() as usize;
                let end = ref_idx.min(total);
                let start = end.saturating_sub(abs_count);
                let first = start == 0;
                let last = end >= total;
                (items[start..end].to_vec(), first, last)
            }
        }
        Some(RangeSpec::ByTime { .. }) => {
            // Time-based filtering requires log record timestamps that aren't
            // available through the property value interface.
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            });
        }
    };

    // Encode selected items, counting only those that encode successfully
    let mut item_data = BytesMut::new();
    let mut encoded_count: u32 = 0;
    for item in &selected {
        if encode_property_value(&mut item_data, item).is_err() {
            continue;
        }
        encoded_count += 1;
    }
    let item_count = encoded_count;

    let ack = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: (first_item, last_item, false),
        item_count,
        item_data: item_data.to_vec(),
        first_sequence_number: None,
    };

    ack.encode(buf);
    Ok(())
}

/// Handle an AtomicReadFile request (Clause 15.1).
pub fn handle_atomic_read_file(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::file::{
        AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod,
    };

    let request = AtomicReadFileRequest::decode(service_data)?;

    let object = db.get(&request.file_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    // Verify it's a File object
    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
        });
    }

    // Read the file data via FILE_SIZE and then access raw data
    // We use read_property to get FILE_SIZE for bounds checking
    let file_size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .ok()
        .and_then(|v| match v {
            PropertyValue::Unsigned(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0);

    match request.access {
        FileAccessMethod::Stream {
            file_start_position,
            requested_octet_count,
        } => {
            let start = file_start_position.max(0) as u64;
            let count = requested_octet_count as u64;
            let end_of_file = start + count >= file_size;

            // Read actual data via FILE_DATA property (OctetString)
            let file_data = object
                .read_property(PropertyIdentifier::from_raw(PROP_FILE_DATA), None) // Not standard — fallback
                .ok()
                .and_then(|v| match v {
                    PropertyValue::OctetString(d) => Some(d),
                    _ => None,
                })
                .unwrap_or_default();

            let s = start as usize;
            let e = (s + count as usize).min(file_data.len());
            let data = if s < file_data.len() {
                file_data[s..e].to_vec()
            } else {
                Vec::new()
            };

            let ack = AtomicReadFileAck {
                end_of_file,
                access: FileReadAckMethod::Stream {
                    file_start_position,
                    file_data: data,
                },
            };
            ack.encode(buf);
            Ok(())
        }
        FileAccessMethod::Record {
            file_start_record,
            requested_record_count,
        } => {
            let start = file_start_record.max(0) as usize;
            let count = requested_record_count as usize;

            // Read RECORD_COUNT
            let record_count = object
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Unsigned(n) => Some(n as usize),
                    _ => None,
                })
                .unwrap_or(0);

            let end = (start + count).min(record_count);
            let end_of_file = end >= record_count;

            // Read records by reading FILE_DATA which returns list
            let records_data: Vec<Vec<u8>> = (start..end)
                .map(|_| {
                    // Each record would need individual access; return empty for now
                    Vec::new()
                })
                .collect();

            let ack = AtomicReadFileAck {
                end_of_file,
                access: FileReadAckMethod::Record {
                    file_start_record,
                    returned_record_count: records_data.len() as u32,
                    file_record_data: records_data,
                },
            };
            ack.encode(buf);
            Ok(())
        }
    }
}

/// Handle an AtomicWriteFile request (Clause 15.2).
pub fn handle_atomic_write_file(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    use bacnet_services::file::{
        AtomicWriteFileAck, AtomicWriteFileRequest, FileWriteAccessMethod, FileWriteAckMethod,
    };

    let request = AtomicWriteFileRequest::decode(service_data)?;

    // Verify File object exists
    let object = db.get(&request.file_identifier).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
        });
    }

    // Check read-only
    let read_only = object
        .read_property(PropertyIdentifier::READ_ONLY, None)
        .ok()
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(b),
            _ => None,
        })
        .unwrap_or(false);

    if read_only {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
        });
    }

    match request.access {
        FileWriteAccessMethod::Stream {
            file_start_position,
            file_data,
        } => {
            // Write file data at position — for now store via write_property if possible
            let object = db
                .get_mut(&request.file_identifier)
                .ok_or(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                })?;

            // Read existing data, extend if needed, and write back
            let mut existing = object
                .read_property(PropertyIdentifier::from_raw(PROP_FILE_DATA), None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::OctetString(d) => Some(d),
                    _ => None,
                })
                .unwrap_or_default();

            let start = file_start_position.max(0) as usize;
            if start + file_data.len() > existing.len() {
                existing.resize(start + file_data.len(), 0);
            }
            existing[start..start + file_data.len()].copy_from_slice(&file_data);

            // Update via write_property (OctetString)
            object.write_property(
                PropertyIdentifier::from_raw(PROP_FILE_DATA),
                None,
                PropertyValue::OctetString(existing),
                None,
            )?;

            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Stream {
                    file_start_position,
                },
            };
            ack.encode(buf);
            Ok(())
        }
        FileWriteAccessMethod::Record {
            file_start_record, ..
        } => {
            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Record { file_start_record },
            };
            ack.encode(buf);
            Ok(())
        }
    }
}

/// Handle an AddListElement request (Clause 15.3.1).
///
/// Reads the target property, appends the new elements, and writes back.
/// Returns Ok(()) for a SimpleACK response, or Err for protocol errors.
pub fn handle_add_list_element(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    use bacnet_encoding::primitives::decode_application_value;
    use bacnet_services::list_manipulation::ListElementRequest;

    let request = ListElementRequest::decode(service_data)?;

    let object = db
        .get_mut(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    // Read current list
    let current =
        object.read_property(request.property_identifier, request.property_array_index)?;
    let mut items = match current {
        PropertyValue::List(items) => items,
        _ => Vec::new(),
    };

    // Decode elements from raw bytes
    let mut offset = 0;
    let data = &request.list_of_elements;
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((val, new_offset)) => {
                items.push(val);
                offset = new_offset;
            }
            Err(_) => break,
        }
    }

    // Write back
    object.write_property(
        request.property_identifier,
        request.property_array_index,
        PropertyValue::List(items),
        None,
    )?;

    Ok(())
}

/// Handle a RemoveListElement request (Clause 15.3.2).
///
/// Reads the target property, removes matching elements, and writes back.
pub fn handle_remove_list_element(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<(), Error> {
    use bacnet_encoding::primitives::decode_application_value;
    use bacnet_services::list_manipulation::ListElementRequest;

    let request = ListElementRequest::decode(service_data)?;

    let object = db
        .get_mut(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    // Read current list
    let current =
        object.read_property(request.property_identifier, request.property_array_index)?;
    let mut items = match current {
        PropertyValue::List(items) => items,
        _ => Vec::new(),
    };

    // Decode elements to remove
    let mut to_remove = Vec::new();
    let mut offset = 0;
    let data = &request.list_of_elements;
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((val, new_offset)) => {
                to_remove.push(val);
                offset = new_offset;
            }
            Err(_) => break,
        }
    }

    // Remove matching elements
    items.retain(|item| !to_remove.contains(item));

    // Write back
    object.write_property(
        request.property_identifier,
        request.property_array_index,
        PropertyValue::List(items),
        None,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::traits::BACnetObject;

    fn make_db_with_ai() -> ObjectDatabase {
        let mut db = ObjectDatabase::new();
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.set_present_value(72.5);
        db.add(Box::new(ai)).unwrap();
        db
    }

    #[test]
    fn read_property_handler_success() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = ReadPropertyACK::decode(&ack_bytes).unwrap();
        assert_eq!(ack.object_identifier, oid);
        assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);

        // Decode the value
        let (val, _) =
            bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
        assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
    }

    #[test]
    fn read_property_unknown_object() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        let request = ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_read_property(&db, &buf, &mut ack_buf);
        assert!(result.is_err());
    }

    #[test]
    fn read_property_unknown_property() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_read_property(&db, &buf, &mut ack_buf);
        assert!(result.is_err());
    }

    #[test]
    fn write_property_handler_success() {
        let mut db = ObjectDatabase::new();
        let bv = bacnet_objects::binary::BinaryValueObject::new(1, "BV-1").unwrap();
        db.add(Box::new(bv)).unwrap();

        let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

        // Encode write request: set present-value to active (1)
        let mut value_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);

        let request = WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: value_buf.to_vec(),
            priority: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        handle_write_property(&mut db, &buf).unwrap();

        // Verify the value was written
        let obj = db.get(&oid).unwrap();
        let val = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, bacnet_types::primitives::PropertyValue::Enumerated(1));
    }

    #[test]
    fn rpm_handler_success() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        use bacnet_services::common::PropertyReference;
        use bacnet_services::rpm::ReadAccessSpecification;

        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

        assert_eq!(ack.list_of_read_access_results.len(), 1);
        let result = &ack.list_of_read_access_results[0];
        assert_eq!(result.object_identifier, oid);
        assert_eq!(result.list_of_results.len(), 2);

        // Both should be successful
        assert!(result.list_of_results[0].property_value.is_some());
        assert!(result.list_of_results[1].property_value.is_some());

        // Verify present-value is Real(72.5)
        let (val, _) = bacnet_encoding::primitives::decode_application_value(
            result.list_of_results[0].property_value.as_ref().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
    }

    #[test]
    fn rpm_handler_unknown_property_returns_inline_error() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        use bacnet_services::common::PropertyReference;
        use bacnet_services::rpm::ReadAccessSpecification;

        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                        property_array_index: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

        let result = &ack.list_of_read_access_results[0];
        assert!(result.list_of_results[0].property_value.is_some()); // present-value ok
        assert!(result.list_of_results[1].error.is_some()); // priority-array unknown
    }

    #[test]
    fn rpm_handler_unknown_object_returns_inline_error() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        use bacnet_services::common::PropertyReference;
        use bacnet_services::rpm::ReadAccessSpecification;

        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

        let result = &ack.list_of_read_access_results[0];
        assert!(result.list_of_results[0].error.is_some());
    }

    #[test]
    fn rpm_handler_all_properties_expanded() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        use bacnet_services::common::PropertyReference;
        use bacnet_services::rpm::ReadAccessSpecification;

        let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::ALL,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

        assert_eq!(ack.list_of_read_access_results.len(), 1);
        let result = &ack.list_of_read_access_results[0];
        assert_eq!(result.object_identifier, oid);

        // AnalogInputObject.property_list() returns multiple properties
        let obj = db.get(&oid).unwrap();
        let expected_props = obj.property_list();
        assert!(
            expected_props.len() > 2,
            "sanity: AI should have many properties"
        );
        assert_eq!(result.list_of_results.len(), expected_props.len());

        // Verify each result matches the expected property identifier
        for (elem, &expected_pid) in result.list_of_results.iter().zip(expected_props.iter()) {
            assert_eq!(elem.property_identifier, expected_pid);
        }

        // Verify present-value is included and correct
        let pv_elem = result
            .list_of_results
            .iter()
            .find(|e| e.property_identifier == PropertyIdentifier::PRESENT_VALUE)
            .expect("PRESENT_VALUE should be in ALL results");
        assert!(pv_elem.property_value.is_some());
    }

    #[test]
    fn rpm_handler_required_vs_optional() {
        let db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        use bacnet_services::common::PropertyReference;
        use bacnet_services::rpm::ReadAccessSpecification;

        // REQUIRED wildcard
        let req_required = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::REQUIRED,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req_required.encode(&mut buf);
        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();
        let required_results = &ack.list_of_read_access_results[0].list_of_results;

        // OPTIONAL wildcard
        let req_optional = bacnet_services::rpm::ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::OPTIONAL,
                    property_array_index: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req_optional.encode(&mut buf);
        let mut ack_buf = BytesMut::new();
        handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();
        let optional_results = &ack.list_of_read_access_results[0].list_of_results;

        // REQUIRED must include the 4 universal properties
        let req_pids: Vec<_> = required_results
            .iter()
            .map(|r| r.property_identifier)
            .collect();
        assert!(req_pids.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
        assert!(req_pids.contains(&PropertyIdentifier::OBJECT_NAME));
        assert!(req_pids.contains(&PropertyIdentifier::OBJECT_TYPE));
        assert!(req_pids.contains(&PropertyIdentifier::PROPERTY_LIST));

        // OPTIONAL must NOT include any required properties
        let opt_pids: Vec<_> = optional_results
            .iter()
            .map(|r| r.property_identifier)
            .collect();
        for req_pid in &req_pids {
            assert!(
                !opt_pids.contains(req_pid),
                "OPTIONAL should not contain {req_pid:?}"
            );
        }

        // REQUIRED + OPTIONAL should cover ALL.
        // Note: REQUIRED may include PROPERTY_LIST (per Clause 12.11.12,
        // property_list() excludes itself, so REQUIRED can have 1 extra).
        let obj = db.get(&oid).unwrap();
        let all_pids = obj.property_list();
        let required_set: std::collections::HashSet<_> = req_pids.iter().collect();
        let optional_set: std::collections::HashSet<_> = opt_pids.iter().collect();
        for pid in all_pids.iter() {
            assert!(
                required_set.contains(pid) || optional_set.contains(pid),
                "ALL property {pid:?} missing from REQUIRED and OPTIONAL"
            );
        }
    }

    #[test]
    fn wpm_handler_success() {
        let mut db = ObjectDatabase::new();
        let bv = bacnet_objects::binary::BinaryValueObject::new(1, "BV-1").unwrap();
        db.add(Box::new(bv)).unwrap();

        let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

        let mut value_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);

        use bacnet_services::common::BACnetPropertyValue;
        use bacnet_services::wpm::WriteAccessSpecification;

        let request = bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: value_buf.to_vec(),
                    priority: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        handle_write_property_multiple(&mut db, &buf).unwrap();

        let obj = db.get(&oid).unwrap();
        let val = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, bacnet_types::primitives::PropertyValue::Enumerated(1));
    }

    #[test]
    fn subscribe_cov_handler_success() {
        let db = make_db_with_ai();
        let mut table = CovSubscriptionTable::new();
        let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn subscribe_cov_unknown_object_fails() {
        let db = make_db_with_ai();
        let mut table = CovSubscriptionTable::new();
        let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        let request = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        assert!(handle_subscribe_cov(&mut table, &db, &mac, &buf).is_err());
        assert!(table.is_empty());
    }

    #[test]
    fn subscribe_cov_cancellation() {
        let db = make_db_with_ai();
        let mut table = CovSubscriptionTable::new();
        let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        // First subscribe
        let request = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: Some(false),
            lifetime: Some(300),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
        assert_eq!(table.len(), 1);

        // Then cancel
        let cancel = SubscribeCOVRequest {
            subscriber_process_identifier: 1,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: None,
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        cancel.encode(&mut buf);
        handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
        assert!(table.is_empty());
    }

    #[test]
    fn who_has_by_id_found() {
        let db = make_db_with_ai();
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Identifier(ai_oid),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let result = handle_who_has(&db, &buf, device_oid).unwrap();
        assert!(result.is_some());
        let i_have = result.unwrap();
        assert_eq!(i_have.object_identifier, ai_oid);
        assert_eq!(i_have.object_name, "AI-1");
    }

    #[test]
    fn who_has_by_name_found() {
        let db = make_db_with_ai();
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();

        let request = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Name("AI-1".into()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let result = handle_who_has(&db, &buf, device_oid).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn who_has_not_found() {
        let db = make_db_with_ai();
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let missing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        let request = WhoHasRequest {
            low_limit: None,
            high_limit: None,
            object: WhoHasObject::Identifier(missing_oid),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let result = handle_who_has(&db, &buf, device_oid).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn who_has_out_of_range() {
        let db = make_db_with_ai();
        let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = WhoHasRequest {
            low_limit: Some(100),
            high_limit: Some(200),
            object: WhoHasObject::Identifier(ai_oid),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let result = handle_who_has(&db, &buf, device_oid).unwrap();
        assert!(result.is_none()); // device instance 1 not in [100, 200]
    }

    #[test]
    fn delete_object_success() {
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = bacnet_services::object_mgmt::DeleteObjectRequest {
            object_identifier: oid,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        handle_delete_object(&mut db, &buf).unwrap();
        assert!(db.get(&oid).is_none());
    }

    #[test]
    fn delete_object_unknown_fails() {
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        let request = bacnet_services::object_mgmt::DeleteObjectRequest {
            object_identifier: oid,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        assert!(handle_delete_object(&mut db, &buf).is_err());
    }

    #[test]
    fn delete_device_object_fails() {
        let mut db = ObjectDatabase::new();
        let device =
            bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
                instance: 1,
                name: "Dev".into(),
                ..Default::default()
            })
            .unwrap();
        db.add(Box::new(device)).unwrap();

        let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let request = bacnet_services::object_mgmt::DeleteObjectRequest {
            object_identifier: oid,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        assert!(handle_delete_object(&mut db, &buf).is_err());
    }

    #[test]
    fn device_communication_control_handler() {
        let comm_state = AtomicU8::new(0);

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: Some(60),
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, duration) =
            handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(state, EnableDisable::DISABLE_INITIATION);
        assert_eq!(duration, Some(60));
        assert_eq!(comm_state.load(Ordering::Acquire), 2);
    }

    #[test]
    fn device_communication_control_enable() {
        let comm_state = AtomicU8::new(1); // start disabled

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::ENABLE,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, duration) =
            handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(state, EnableDisable::ENABLE);
        assert_eq!(duration, None);
        assert_eq!(comm_state.load(Ordering::Acquire), 0);
    }

    #[test]
    fn device_communication_control_disable_initiation() {
        let comm_state = AtomicU8::new(0);

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, duration) =
            handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(state, EnableDisable::DISABLE_INITIATION);
        assert_eq!(duration, None);
        assert_eq!(comm_state.load(Ordering::Acquire), 2);
    }

    #[test]
    fn reinitialize_device_handler() {
        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        handle_reinitialize_device(&buf, &None).unwrap();
    }

    #[test]
    fn get_event_information_empty() {
        let db = make_db_with_ai();
        let request = bacnet_services::alarm_event::GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        assert!(!ack_bytes.is_empty());
    }

    #[test]
    fn get_event_information_reports_non_normal_objects() {
        use bacnet_objects::event::LimitEnable;

        let mut db = ObjectDatabase::new();
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        // Configure intrinsic reporting
        ai.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
        // Push value above high limit and evaluate
        ai.set_present_value(85.0);
        ai.evaluate_intrinsic_reporting(); // → HIGH_LIMIT
        db.add(Box::new(ai)).unwrap();

        let request = bacnet_services::alarm_event::GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        // The ACK should contain one event summary for AI-1
        assert!(ack_bytes.len() > 5); // non-trivial response
    }

    #[test]
    fn get_event_information_reads_event_enable_and_notify_type() {
        use bacnet_objects::event::LimitEnable;
        use bacnet_objects::notification_class::NotificationClass;

        let mut db = ObjectDatabase::new();
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(80.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LOW_LIMIT,
            None,
            PropertyValue::Real(20.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::DEADBAND,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::BOTH.to_bits()],
            },
            None,
        )
        .unwrap();
        // Set EVENT_ENABLE to 0x05 (TO_OFFNORMAL + TO_NORMAL, not TO_FAULT)
        ai.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x05 << 5],
            },
            None,
        )
        .unwrap();
        // Set NOTIFY_TYPE to 1 (EVENT, not ALARM)
        ai.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
        // Push above high limit and evaluate to trigger alarm
        ai.set_present_value(85.0);
        ai.evaluate_intrinsic_reporting();
        db.add(Box::new(ai)).unwrap();

        // Add NotificationClass object with custom priorities
        let mut nc = NotificationClass::new(5, "NC-5").unwrap();
        nc.priority = [100, 150, 200];
        db.add(Box::new(nc)).unwrap();

        let request = bacnet_services::alarm_event::GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_get_event_information(&db, &buf, &mut ack_buf).unwrap();
        let ack_bytes = ack_buf.to_vec();
        assert!(
            ack_bytes.len() > 10,
            "ACK should contain event summary data"
        );

        // Verify the object reads EVENT_ENABLE=0x05 and NOTIFY_TYPE=1 (EVENT)
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ai_ref = db.get(&ai_oid).unwrap();
        let ev_enable = ai_ref
            .read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap();
        match ev_enable {
            PropertyValue::BitString { data, .. } => {
                assert_eq!(data[0] >> 5, 0x05, "EVENT_ENABLE should be 0x05");
            }
            other => panic!("expected BitString, got {:?}", other),
        }
        let notify = ai_ref
            .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap();
        assert_eq!(
            notify,
            PropertyValue::Enumerated(1),
            "NOTIFY_TYPE should be EVENT(1)"
        );
    }

    #[test]
    fn wpm_handler_unknown_object_fails() {
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 99).unwrap();

        use bacnet_services::common::BACnetPropertyValue;
        use bacnet_services::wpm::WriteAccessSpecification;

        let request = bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x91, 0x01],
                    priority: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        assert!(handle_write_property_multiple(&mut db, &buf).is_err());
    }

    #[test]
    fn wpm_handler_atomicity_rollback() {
        // Write two properties: first succeeds (HIGH_LIMIT), second fails (read-only OBJECT_TYPE).
        // Verify HIGH_LIMIT is rolled back to its original value.
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let original_hl = match db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::HIGH_LIMIT, None)
            .unwrap()
        {
            PropertyValue::Real(v) => v,
            _ => panic!("expected Real"),
        };

        let mut hl_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_real(&mut hl_buf, 999.0);
        let mut ot_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

        use bacnet_services::common::BACnetPropertyValue;
        use bacnet_services::wpm::WriteAccessSpecification;

        let request = bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::HIGH_LIMIT,
                        property_array_index: None,
                        value: hl_buf.to_vec(),
                        priority: None,
                    },
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::OBJECT_TYPE,
                        property_array_index: None,
                        value: ot_buf.to_vec(),
                        priority: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        // Should fail because OBJECT_TYPE is read-only
        assert!(handle_write_property_multiple(&mut db, &buf).is_err());

        // HIGH_LIMIT should be rolled back to original
        let after_hl = match db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::HIGH_LIMIT, None)
            .unwrap()
        {
            PropertyValue::Real(v) => v,
            _ => panic!("expected Real"),
        };
        assert_eq!(
            original_hl, after_hl,
            "HIGH_LIMIT should be rolled back after failed WPM"
        );
    }

    fn make_db_with_device_and_ai() -> ObjectDatabase {
        let mut db = ObjectDatabase::new();
        let device =
            bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
                instance: 1,
                name: "TestDevice".into(),
                ..Default::default()
            })
            .unwrap();
        db.add(Box::new(device)).unwrap();
        db.add(Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()))
            .unwrap();
        db
    }

    #[test]
    fn create_object_by_type_assigns_next_instance() {
        let mut db = make_db_with_device_and_ai();
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_create_object(&mut db, &buf, &mut ack_buf);
        assert!(result.is_ok());
        // Should now have 3 objects (Device + AI-1 + new AI)
        assert_eq!(db.len(), 3);
        // The new AI should have instance 2 (since 1 is taken)
        let ai2_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
        assert!(db.get(&ai2_oid).is_some());
    }

    #[test]
    fn create_object_by_identifier() {
        let mut db = make_db_with_device_and_ai();
        let target_oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 99).unwrap();
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(target_oid),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_create_object(&mut db, &buf, &mut ack_buf);
        assert!(result.is_ok());
        assert!(db.get(&target_oid).is_some());
    }

    #[test]
    fn create_object_duplicate_fails() {
        let mut db = make_db_with_device_and_ai();
        let existing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(existing_oid),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_create_object(&mut db, &buf, &mut ack_buf);
        assert!(result.is_err());
    }

    #[test]
    fn create_unsupported_type_fails() {
        let mut db = make_db_with_device_and_ai();
        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::DEVICE),
            list_of_initial_values: vec![],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        let result = handle_create_object(&mut db, &buf, &mut ack_buf);
        assert!(result.is_err());
    }

    #[test]
    fn create_object_with_initial_values() {
        let mut db = make_db_with_device_and_ai();
        let mut desc_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_character_string(&mut desc_buf, "Test AI").unwrap();

        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: desc_buf.to_vec(),
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        let mut ack_buf = BytesMut::new();
        handle_create_object(&mut db, &buf, &mut ack_buf).unwrap();
        let (pv, _) = bacnet_encoding::primitives::decode_application_value(&ack_buf, 0).unwrap();
        let created_oid = match pv {
            PropertyValue::ObjectIdentifier(oid) => oid,
            other => panic!("expected ObjectIdentifier, got {other:?}"),
        };

        let obj = db.get(&created_oid).unwrap();
        let desc = obj
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        match desc {
            PropertyValue::CharacterString(s) => assert_eq!(s, "Test AI"),
            other => panic!("expected CharacterString, got {other:?}"),
        }
    }

    #[test]
    fn create_object_bad_initial_value_rolls_back() {
        let mut db = make_db_with_device_and_ai();
        let before_count = db.len();

        // Try to write OBJECT_TYPE (read-only) as an initial value
        let mut ot_buf = BytesMut::new();
        bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 99);

        let req = CreateObjectRequest {
            object_specifier: ObjectSpecifier::Type(ObjectType::BINARY_INPUT),
            list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_TYPE,
                property_array_index: None,
                value: ot_buf.to_vec(),
                priority: None,
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);

        assert!(handle_create_object(&mut db, &buf, &mut BytesMut::new()).is_err());
        assert_eq!(
            db.len(),
            before_count,
            "object should be removed on failure"
        );
    }

    // -----------------------------------------------------------------------
    // AcknowledgeAlarm handler tests
    // -----------------------------------------------------------------------

    #[test]
    fn acknowledge_alarm_success() {
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: oid,
            event_state_acknowledged: 3,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
            time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        handle_acknowledge_alarm(&mut db, &buf).unwrap();
    }

    #[test]
    fn acknowledge_alarm_unknown_object_fails() {
        let mut db = make_db_with_ai();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 1,
            event_object_identifier: oid,
            event_state_acknowledged: 3,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
            time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let err = handle_acknowledge_alarm(&mut db, &buf).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::OBJECT.to_raw() as u32);
                assert_eq!(code, ErrorCode::UNKNOWN_OBJECT.to_raw() as u32);
            }
            other => panic!("expected Protocol error, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // DCC timer auto-re-enable tests (Clause 16.4.3)
    // -----------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn dcc_timer_auto_re_enables() {
        use std::sync::Arc;

        let comm_state = Arc::new(AtomicU8::new(0));

        // Send DCC DISABLE with 1-minute duration
        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: Some(1), // 1 minute
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, duration) =
            handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(state, EnableDisable::DISABLE_INITIATION);
        assert_eq!(duration, Some(1));
        assert_eq!(comm_state.load(Ordering::Acquire), 2);

        // Simulate what the server dispatch does: spawn a timer task
        let comm_clone = Arc::clone(&comm_state);
        let minutes = duration.unwrap();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(minutes as u64 * 60)).await;
            comm_clone.store(0, Ordering::Release);
        });

        // State should still be DISABLE before timer fires
        assert_eq!(comm_state.load(Ordering::Acquire), 2);

        // Advance time past the 1-minute duration
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        // Wait for the spawned task to complete (which sets state back to 0)
        handle.await.unwrap();

        // State should now be re-enabled
        assert_eq!(comm_state.load(Ordering::Acquire), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dcc_timer_cancelled_by_new_dcc() {
        use std::sync::Arc;
        use tokio::task::JoinHandle;

        let comm_state = Arc::new(AtomicU8::new(0));

        // Send DCC DISABLE with 2-minute duration
        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: Some(2),
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();
        let (_, duration) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(comm_state.load(Ordering::Acquire), 2);

        // Spawn first timer
        let comm_clone = Arc::clone(&comm_state);
        let minutes = duration.unwrap();
        let handle1: JoinHandle<()> = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(minutes as u64 * 60)).await;
            comm_clone.store(0, Ordering::Release);
        });

        // Now send DCC ENABLE (no duration) — should cancel the timer
        let request2 = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::ENABLE,
            password: None,
        };
        let mut buf2 = BytesMut::new();
        request2.encode(&mut buf2).unwrap();
        let (state2, duration2) =
            handle_device_communication_control(&buf2, &comm_state, &None).unwrap();
        assert_eq!(state2, EnableDisable::ENABLE);
        assert_eq!(duration2, None);
        assert_eq!(comm_state.load(Ordering::Acquire), 0);

        // Abort previous timer (simulating server dispatch behavior)
        handle1.abort();

        // Advance past the original 2-minute duration
        tokio::time::advance(std::time::Duration::from_secs(121)).await;
        tokio::task::yield_now().await;

        // State should still be ENABLE (timer was cancelled)
        assert_eq!(comm_state.load(Ordering::Acquire), 0);
    }

    // -----------------------------------------------------------------------
    // Password validation tests (Clause 16.4.1 / 16.4.2)
    // -----------------------------------------------------------------------

    #[test]
    fn dcc_correct_password_accepted() {
        let comm_state = AtomicU8::new(0);
        let pw = Some("secret".to_string());

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: Some("secret".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, _) = handle_device_communication_control(&buf, &comm_state, &pw).unwrap();
        assert_eq!(state, EnableDisable::DISABLE_INITIATION);
        assert_eq!(comm_state.load(Ordering::Acquire), 2);
    }

    #[test]
    fn dcc_wrong_password_rejected() {
        let comm_state = AtomicU8::new(0);
        let pw = Some("secret".to_string());

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: Some("wrong".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let err = handle_device_communication_control(&buf, &comm_state, &pw).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
                assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
            }
            other => panic!("expected Protocol error, got: {other:?}"),
        }
        // State unchanged
        assert_eq!(comm_state.load(Ordering::Acquire), 0);
    }

    #[test]
    fn dcc_missing_password_when_required() {
        let comm_state = AtomicU8::new(0);
        let pw = Some("secret".to_string());

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let err = handle_device_communication_control(&buf, &comm_state, &pw).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
                assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
            }
            other => panic!("expected Protocol error, got: {other:?}"),
        }
        assert_eq!(comm_state.load(Ordering::Acquire), 0);
    }

    #[test]
    fn dcc_no_password_configured_accepts_any() {
        let comm_state = AtomicU8::new(0);

        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::DISABLE_INITIATION,
            password: Some("anything".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let (state, _) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
        assert_eq!(state, EnableDisable::DISABLE_INITIATION);
        assert_eq!(comm_state.load(Ordering::Acquire), 2);
    }

    #[test]
    fn reinit_correct_password_accepted() {
        let pw = Some("reinit-pw".to_string());

        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
            password: Some("reinit-pw".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        handle_reinitialize_device(&buf, &pw).unwrap();
    }

    #[test]
    fn reinit_wrong_password_rejected() {
        let pw = Some("reinit-pw".to_string());

        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
            password: Some("wrong".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let err = handle_reinitialize_device(&buf, &pw).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
                assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
            }
            other => panic!("expected Protocol error, got: {other:?}"),
        }
    }

    #[test]
    fn reinit_missing_password_when_required() {
        let pw = Some("reinit-pw".to_string());

        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
            password: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        let err = handle_reinitialize_device(&buf, &pw).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
                assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
            }
            other => panic!("expected Protocol error, got: {other:?}"),
        }
    }

    #[test]
    fn reinit_no_password_configured_accepts_any() {
        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
            password: Some("anything".to_string()),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();

        handle_reinitialize_device(&buf, &None).unwrap();
    }
}
