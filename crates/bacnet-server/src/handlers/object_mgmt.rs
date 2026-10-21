use super::*;
use bacnet_types::enums::EngineeringUnits;

/// Handle a WhoHas request and return an IHave response if we have the object.
///
/// Returns `Some(IHaveRequest)` if we have the requested object, `None` otherwise.
pub fn handle_who_has(
    db: &ObjectDatabase,
    service_data: &[u8],
    device_oid: ObjectIdentifier,
) -> Result<Option<IHaveRequest>, Error> {
    let request = WhoHasRequest::decode(service_data)?;

    let instance = device_oid.instance_number();
    if request.range.is_some_and(|range| !range.contains(instance)) {
        return Ok(None);
    }

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
/// A refused initial value is `Error::Structured` naming its position in the
/// List of Initial Values (Clause 15.3.1.3); any other refusal is
/// `Error::Protocol` or a decoding error.
pub fn handle_create_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    handle_create_object_observed(db, service_data, buf, &mut None, None)
        .map_err(CreateObjectRefusal::into_error)
}

/// Why the CreateObject handler refused a request.
#[derive(Debug)]
pub(crate) enum CreateObjectRefusal {
    /// The request, or one of its initial values, does not decode: an invalid
    /// request rather than an executed one, so it is not audited.
    Malformed(Error),
    /// The object could not be created or initialized.
    Failed(Error),
}

impl CreateObjectRefusal {
    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::Malformed(error) | Self::Failed(error) => error,
        }
    }

    /// The refusal of the initial value at `position` (from 1), which names
    /// it.
    fn of_initial_value(self, position: u32) -> Self {
        let named = |error| match error {
            Error::Protocol { class, code } | Error::Structured { class, code, .. } => {
                Error::protocol(
                    class,
                    code,
                    Some(ErrorDetail::FirstFailedElementNumber(position)),
                )
            }
            other => other,
        };
        match self {
            Self::Malformed(error) => Self::Malformed(named(error)),
            Self::Failed(error) => Self::Failed(named(error)),
        }
    }
}

/// Preserve the public handler's result while exposing only the actual requested,
/// allocated candidate, or final identity. Before by-type allocation it is absent.
pub(crate) fn handle_create_object_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    target: &mut Option<ObjectIdentifier>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Result<(), CreateObjectRefusal> {
    *target = None;
    let request = CreateObjectRequest::decode(service_data)
        .map_err(|error| CreateObjectRefusal::Malformed(error.into_request_reject()))?;
    if let ObjectSpecifier::Identifier(oid) = request.object_specifier {
        *target = Some(oid);
    }
    let created_oid = create(db, &request, target).map_err(CreateObjectRefusal::Failed)?;
    let counts = StateCounts::of(db, created_oid, &request);

    // Apply initial values; on failure, remove the created object.
    for (position, pv) in apply_counts_first(db, created_oid, &request, counts, command_origin) {
        if let Err(refusal) = initialize(db, created_oid, pv, counts, command_origin) {
            let _ = db.remove(&created_oid);
            return Err(refusal.of_initial_value(position));
        }
    }

    bacnet_encoding::primitives::encode_app_object_id(buf, &created_oid);
    Ok(())
}

/// Which initial values give a multi-state object its state count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateCounts {
    /// The object has no state count to give (any other type).
    None,
    /// The request's Number_Of_States values give it (#1429). A State_Text
    /// written whole then has to label exactly those states.
    Given,
    /// The request has no Number_Of_States, so State_Text written whole
    /// gives the count, as a WriteProperty of it would (#1443).
    FromStateText,
}

impl StateCounts {
    fn of(
        db: &ObjectDatabase,
        created_oid: ObjectIdentifier,
        request: &CreateObjectRequest,
    ) -> Self {
        let object = db.get(&created_oid).expect("created above");
        if !object
            .creation_only_properties()
            .contains(&PropertyIdentifier::NUMBER_OF_STATES)
        {
            Self::None
        } else if request
            .list_of_initial_values
            .iter()
            .any(|pv| pv.property_identifier == PropertyIdentifier::NUMBER_OF_STATES)
        {
            Self::Given
        } else {
            Self::FromStateText
        }
    }

    /// Whether `pv` is one of the values that give the count.
    fn gives(self, pv: &bacnet_services::common::BACnetPropertyValue) -> bool {
        match self {
            Self::None => false,
            Self::Given => pv.property_identifier == PropertyIdentifier::NUMBER_OF_STATES,
            Self::FromStateText => {
                pv.property_identifier == PropertyIdentifier::STATE_TEXT
                    && pv.property_array_index.is_none()
            }
        }
    }
}

/// Apply the initial values that give a multi-state object its state count
/// first ([`StateCounts`]), and return the initial values left to apply,
/// with their positions, in request order (#1429, #1443).
///
/// Present_Value, Relinquish_Default, Alarm_Values, the State_Text elements
/// and State_Text written whole are then judged against the count the
/// request asks for, wherever it stands in the list. A count value that
/// fails its own checks (an index, its datatype, its range) isn't applied
/// here: it stays in its place among the rest, so a bad value before it is
/// still the one named. With several, each that passes is applied in
/// request order, so the last of those sets the count. A refusal always
/// names the value's own position. On a fresh object a count can only fail
/// its own checks here, since every state the object holds is 1.
fn apply_counts_first<'a>(
    db: &mut ObjectDatabase,
    created_oid: ObjectIdentifier,
    request: &'a CreateObjectRequest,
    counts: StateCounts,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Vec<(u32, &'a bacnet_services::common::BACnetPropertyValue)> {
    // A refused attempt leaves the object as it was, and the value is tried
    // again in its place.
    (1u32..)
        .zip(&request.list_of_initial_values)
        .filter(|(_, pv)| {
            !counts.gives(pv) || initialize(db, created_oid, pv, counts, command_origin).is_err()
        })
        .collect()
}

/// Create the requested object with its default values, setting `target` to
/// its identifier once one is known.
fn create(
    db: &mut ObjectDatabase,
    request: &CreateObjectRequest,
    target: &mut Option<ObjectIdentifier>,
) -> Result<ObjectIdentifier, Error> {
    const MAX_OBJECTS: usize = 10_000;
    if db.len() >= MAX_OBJECTS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES,
            code: ErrorCode::NO_SPACE_FOR_OBJECT,
        });
    }

    let (object_type, instance) = match &request.object_specifier {
        ObjectSpecifier::Type(obj_type) => {
            let existing: HashSet<u32> = db
                .find_by_type(*obj_type)
                .iter()
                .map(|oid| oid.instance_number())
                .collect();
            let next =
                (1u32..=4_194_303)
                    .find(|i| !existing.contains(i))
                    .ok_or(Error::Protocol {
                        class: ErrorClass::RESOURCES,
                        code: ErrorCode::NO_SPACE_FOR_OBJECT,
                    })?;
            (*obj_type, next)
        }
        ObjectSpecifier::Identifier(oid) => {
            if db.get(oid).is_some() {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT,
                    code: ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
                });
            }
            (oid.object_type(), oid.instance_number())
        }
    };

    // Unsupported extensible types need not fit an ObjectIdentifier. Do not
    // truncate them or manufacture an instance when allocation did not occur.
    *target = (object_type.to_raw() <= 1023)
        .then(|| ObjectIdentifier::new(object_type, instance).ok())
        .flatten();
    // Named only once the type is known to be one the server builds.
    let name = || default_name(db, object_type, instance);

    let object: Box<dyn bacnet_objects::traits::BACnetObject> = if object_type
        == ObjectType::ANALOG_INPUT
    {
        Box::new(bacnet_objects::analog::AnalogInputObject::new(
            instance,
            name(),
            EngineeringUnits::NO_UNITS,
        )?)
    } else if object_type == ObjectType::ANALOG_OUTPUT {
        Box::new(bacnet_objects::analog::AnalogOutputObject::new(
            instance,
            name(),
            EngineeringUnits::NO_UNITS,
        )?)
    } else if object_type == ObjectType::BINARY_INPUT {
        Box::new(bacnet_objects::binary::BinaryInputObject::new(
            instance,
            name(),
        )?)
    } else if object_type == ObjectType::BINARY_OUTPUT {
        Box::new(bacnet_objects::binary::BinaryOutputObject::new(
            instance,
            name(),
        )?)
    } else if object_type == ObjectType::BINARY_VALUE {
        let mut object = bacnet_objects::binary::BinaryValueObject::new(instance, name())?;
        // Initial values provision only the optional rows actually requested.
        // Normal WP cannot materialize an absent property. Values still pass
        // the same writer below, including rollback on invalid initialization.
        let mut policy = bacnet_objects::audit::ObjectAuditPolicy::default();
        for value in &request.list_of_initial_values {
            match value.property_identifier {
                PropertyIdentifier::AUDIT_LEVEL => {
                    policy.level = Some(bacnet_types::enums::AuditLevel::DEFAULT)
                }
                PropertyIdentifier::AUDITABLE_OPERATIONS => {
                    policy.operations = Some(bacnet_types::bitstring::AuditOperationFlags::empty())
                }
                PropertyIdentifier::AUDIT_PRIORITY_FILTER => {
                    policy.priority_filter =
                        Some(bacnet_objects::audit::AuditPriorityPolicy::Inherit)
                }
                _ => {}
            }
        }
        object.set_audit_policy(policy);
        Box::new(object)
    } else if object_type == ObjectType::MULTI_STATE_INPUT {
        Box::new(bacnet_objects::multistate::MultiStateInputObject::new(
            instance,
            name(),
            2,
        )?)
    } else if object_type == ObjectType::MULTI_STATE_OUTPUT {
        Box::new(bacnet_objects::multistate::MultiStateOutputObject::new(
            instance,
            name(),
            2,
        )?)
    } else if object_type == ObjectType::MULTI_STATE_VALUE {
        Box::new(bacnet_objects::multistate::MultiStateValueObject::new(
            instance,
            name(),
            2,
        )?)
    } else {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT,
            code: ErrorCode::UNSUPPORTED_OBJECT_TYPE,
        });
    };

    let created_oid = object.object_identifier();
    *target = Some(created_oid);
    db.add(object)?;
    Ok(created_oid)
}

/// The Object_Name a new object starts with until an initial value names
/// it: the type's name and the instance (`BINARY_VALUE-2`), or, when
/// another object already holds that name, the same with the first free
/// ` (n)` from 2 up (`BINARY_VALUE-2 (2)`) (#1437). Clause 15.3 leaves the
/// value of a property the request doesn't give to the device, so a client
/// renaming an object never makes the next create of its type fail.
///
/// Each object holds one name, so one of the first `db.len() + 1`
/// candidates is free.
fn default_name(db: &ObjectDatabase, object_type: ObjectType, instance: u32) -> String {
    let base = format!("{object_type}-{instance}");
    if db.find_by_name(&base).is_none() {
        return base;
    }
    (2u64..)
        .map(|n| format!("{base} ({n})"))
        .find(|name| db.find_by_name(name).is_none())
        .expect("one candidate past the names held is free")
}

/// Apply one initial value to the created object the way WriteProperty
/// applies its value: the array index checked against the object, the value
/// decoded whole with the object's list classification, the Object_Name kept
/// unique, and a NULL the property leaves as it is taken as applied
/// ([`relinquish`]). A whole value for one of the object's
/// [`creation_only_properties`] goes to its
/// [`initialize_property`] instead of the write route, which keeps refusing
/// it (#1429). A State_Text written whole goes the write route, which sets
/// the state count from it (#1443); when the request gives the count
/// itself ([`StateCounts::Given`]), one that would change it is
/// VALUE_OUT_OF_RANGE. The caller removes the object when this fails.
///
/// [`creation_only_properties`]: bacnet_objects::traits::BACnetObject::creation_only_properties
/// [`initialize_property`]: bacnet_objects::traits::BACnetObject::initialize_property
fn initialize(
    db: &mut ObjectDatabase,
    created_oid: ObjectIdentifier,
    pv: &bacnet_services::common::BACnetPropertyValue,
    counts: StateCounts,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Result<(), CreateObjectRefusal> {
    use super::write_property::{
        check_and_prepare_name_write, check_write_array_index, commit_attempt,
        decode_write_property_value, WriteTarget,
    };
    let property = pv.property_identifier;
    let array_index = pv.property_array_index;
    let object = db.get(&created_oid).expect("created above");
    check_write_array_index(object, property, array_index).map_err(CreateObjectRefusal::Failed)?;
    // Octets that don't decode for the property make the request invalid,
    // not a write that was tried and failed.
    let value = decode_write_property_value(
        property,
        array_index,
        object.is_list_property(property),
        &pv.value,
    )
    .map_err(CreateObjectRefusal::Malformed)?;
    let value = crate::local_references::localize(db, created_oid, property, value);
    let object = db.get_mut(&created_oid).expect("created above");
    if array_index.is_none() && object.creation_only_properties().contains(&property) {
        return match object.initialize_property(property, value) {
            Ok(()) => Ok(()),
            // A NULL is judged as on the write route.
            Err(error)
                if super::relinquish::is_null_octets(&pv.value)
                    && super::relinquish::leaves_unchanged(object, property, None, &error) =>
            {
                Ok(())
            }
            Err(error) => Err(CreateObjectRefusal::Failed(error)),
        };
    }
    // The object was added under its default name; a new one has to be free,
    // and the write moves the database's name index along with it.
    if property == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &created_oid, &value)
            .map_err(CreateObjectRefusal::Failed)?;
    }
    let target = WriteTarget {
        oid: created_oid,
        property,
        array_index,
        priority: pv.priority,
        value: &pv.value,
    };
    let fixed_count = (counts == StateCounts::Given
        && property == PropertyIdentifier::STATE_TEXT
        && array_index.is_none())
    .then(|| state_count(db, created_oid));
    commit_attempt(db, None, target, value, None, command_origin)
        .map_err(CreateObjectRefusal::Failed)?;
    // The labels have to fit the count the request gives; the caller drops
    // the object on a refusal, so the resize needs no undoing.
    match fixed_count {
        Some(count) if state_count(db, created_oid) != count => {
            Err(CreateObjectRefusal::Failed(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::VALUE_OUT_OF_RANGE,
            }))
        }
        _ => Ok(()),
    }
}

/// The created object's Number_Of_States, as it reads.
fn state_count(db: &ObjectDatabase, created_oid: ObjectIdentifier) -> Option<PropertyValue> {
    db.get(&created_oid)?
        .read_property(PropertyIdentifier::NUMBER_OF_STATES, None)
        .ok()
}

/// Handle a DeleteObject request.
///
/// Removes the object from the database. Returns an error if the object
/// doesn't exist or is an object type that cannot be deleted at runtime
/// (Device and NetworkPort, which model the running node itself). The set
/// of non-deleteable types is kept in sync with `BACnetObject::is_deleteable`
/// so PICS and runtime dispatch share one truth source.
///
/// Returns the removed object. Drop it after releasing the database guard,
/// and off the async runtime: an object that saves its state (a Notification
/// Forwarder or Audit Log with persistence) waits for its queued saves when
/// dropped (see [`bacnet_objects::durable`]).
pub fn handle_delete_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Box<dyn bacnet_objects::traits::BACnetObject>, Error> {
    let request = DeleteObjectRequest::decode(service_data).map_err(Error::into_request_reject)?;

    match request.object_identifier.object_type() {
        ObjectType::DEVICE | ObjectType::NETWORK_PORT => {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT,
                code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED,
            });
        }
        _ => {}
    }

    db.remove(&request.object_identifier)?
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT,
            code: ErrorCode::UNKNOWN_OBJECT,
        })
}
