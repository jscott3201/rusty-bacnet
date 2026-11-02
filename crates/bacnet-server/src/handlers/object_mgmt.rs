use super::*;
use bacnet_objects::creation::CreationOnly;
use bacnet_types::error::RemoteError;

/// Handle a CreateObject request.
///
/// Supports creating objects by type (server picks instance) or by identifier.
/// Returns the encoded ObjectIdentifier of the created object (ComplexAck payload).
/// A refused initial value is `Error::Structured` naming its position in the
/// List of Initial Values (Clause 15.3.1.3); any other refusal is
/// `Error::protocol` or a decoding error.
///
/// This builds only the types of the built-in table,
/// [`bacnet_objects::creation`]; the running server also builds those of
/// its [`ObjectTypeRegistry`](crate::server::ObjectTypeRegistry).
pub fn handle_create_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let constructors = crate::server::ObjectTypeRegistry::default();
    handle_create_object_observed(db, &constructors, service_data, buf, &mut None, None)
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
            Error::Remote(RemoteError::Error { class, code })
            | Error::Structured { class, code, .. } => Error::with_detail(
                class,
                code,
                Some(ErrorDetail::FirstFailedElementNumber(position)),
            ),
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
    constructors: &crate::server::ObjectTypeRegistry,
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
    let created_oid =
        create(db, constructors, &request, target).map_err(CreateObjectRefusal::Failed)?;
    // What the type takes only at creation (#2179): the registration's
    // answer for a registered type, the built-in table's for the rest.
    let creation_only = constructors.creation_only(created_oid.object_type());
    let counts = StateCounts::of(creation_only, &request);
    let creating = Creating {
        oid: created_oid,
        creation_only,
        counts,
        command_origin,
    };

    // Apply initial values; on failure, remove the created object.
    for (position, pv) in apply_sizes_first(db, &creating, &request) {
        if let Err(refusal) = initialize(db, &creating, pv) {
            let _ = db.remove(&created_oid);
            return Err(refusal.of_initial_value(position));
        }
    }

    bacnet_services::object_mgmt::CreateObjectAck {
        object_identifier: created_oid,
    }
    .encode(buf);
    Ok(())
}

/// The object a CreateObject request is creating, and what its initial
/// values need to know about it.
struct Creating<'a> {
    oid: ObjectIdentifier,
    /// The type's creation-only properties and their initializer.
    creation_only: CreationOnly,
    counts: StateCounts,
    command_origin: Option<&'a bacnet_objects::command_source::CommandOrigin>,
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
    fn of(creation_only: CreationOnly, request: &CreateObjectRequest) -> Self {
        if !creation_only.contains(PropertyIdentifier::NUMBER_OF_STATES) {
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

/// Apply the initial values that size the new object first: those that
/// give a multi-state object its state count ([`StateCounts`]), and a Trend
/// Log's Buffer_Size (#2208). Return the initial values left to apply, with
/// their positions, in request order (#1429, #1443).
///
/// Buffer_Size goes first because a log takes it only while it holds no
/// record, and other initial values (a new Log_DeviceObjectProperty, Enable
/// FALSE) record a status, so the result must not depend on their order.
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
fn apply_sizes_first<'a>(
    db: &mut ObjectDatabase,
    creating: &Creating<'_>,
    request: &'a CreateObjectRequest,
) -> Vec<(u32, &'a bacnet_services::common::BACnetPropertyValue)> {
    // A refused attempt leaves the object as it was, and the value is tried
    // again in its place.
    (1u32..)
        .zip(&request.list_of_initial_values)
        .filter(|(_, pv)| {
            let sizes = creating.counts.gives(pv)
                || (pv.property_identifier == PropertyIdentifier::BUFFER_SIZE
                    && pv.property_array_index.is_none());
            !sizes || initialize(db, creating, pv).is_err()
        })
        .collect()
}

/// Create the requested object with its default values, setting `target` to
/// its identifier once one is known. An application's registered
/// constructor for the type comes before the built-in table's.
fn create(
    db: &mut ObjectDatabase,
    constructors: &crate::server::ObjectTypeRegistry,
    request: &CreateObjectRequest,
    target: &mut Option<ObjectIdentifier>,
) -> Result<ObjectIdentifier, Error> {
    // Clause 15.3.1.3 has the device look for a named object before
    // anything else, so an identifier already in use is refused first,
    // whatever its type: an application's own object of a vendor type too.
    if let ObjectSpecifier::Identifier(oid) = &request.object_specifier
        && db.get(oid).is_some()
    {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
        ));
    }
    // The clause keeps apart a type the device supports but does not create
    // from one it does not support. The Device advertises exactly
    // SUPPORTED_OBJECT_TYPES, and the application supports the proprietary
    // types it registered, so any other type is refused next, whatever
    // instance the request names; only a supported type gets the refusals
    // below, DYNAMIC_CREATION_NOT_SUPPORTED among them.
    let object_type = match &request.object_specifier {
        ObjectSpecifier::Type(object_type) => *object_type,
        ObjectSpecifier::Identifier(oid) => oid.object_type(),
    };
    let registered = constructors.get(object_type);
    if registered.is_none()
        && !bacnet_objects::device::SUPPORTED_OBJECT_TYPES.contains(&object_type)
    {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::UNSUPPORTED_OBJECT_TYPE,
        ));
    }
    const MAX_OBJECTS: usize = 10_000;
    if db.len() >= MAX_OBJECTS {
        return Err(Error::protocol(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_FOR_OBJECT,
        ));
    }

    let (object_type, instance) = match &request.object_specifier {
        ObjectSpecifier::Type(obj_type) => {
            let existing: HashSet<u32> = db
                .find_by_type(*obj_type)
                .iter()
                .map(|oid| oid.instance_number())
                .collect();
            // 4194303 is reserved (Clause 12.1.1), so the last instance the
            // server can pick is 4194302.
            let next = (1u32..=ObjectIdentifier::MAX_ADDRESSABLE_INSTANCE)
                .find(|i| !existing.contains(i))
                .ok_or(Error::protocol(
                    ErrorClass::RESOURCES,
                    ErrorCode::NO_SPACE_FOR_OBJECT,
                ))?;
            (*obj_type, next)
        }
        ObjectSpecifier::Identifier(oid) => {
            // Clause 12.1.1 gives no object the instance 4194303 (it only
            // means "not initialized" in properties), and the Clause 15.3.1.3
            // error table has no row for asking for it. The device can't
            // create such an object, so it answers as for a creation it won't
            // perform; nothing is added.
            if oid.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE {
                return Err(Error::protocol(
                    ErrorClass::OBJECT,
                    ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
                ));
            }
            (oid.object_type(), oid.instance_number())
        }
    };

    // A supported type fits an ObjectIdentifier: a listed one does, and a
    // registration takes no type past 1023.
    let oid = ObjectIdentifier::new(object_type, instance)?;
    *target = Some(oid);
    let object = if let Some(registration) = registered {
        crate::server::construct_registered_object(
            &registration.constructor,
            oid,
            &default_name(db, object_type, instance),
        )?
    } else {
        // A listed type the table doesn't build (#2172).
        let constructor =
            bacnet_objects::creation::builtin_constructor(object_type).ok_or(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
            ))?;
        // The constructor provisions the optional rows the initial values
        // name, since WriteProperty can't add an absent one; the values
        // themselves go through the writer below, with rollback on a refusal.
        let initial_properties: Vec<PropertyIdentifier> = request
            .list_of_initial_values
            .iter()
            .map(|value| value.property_identifier)
            .collect();
        constructor(
            instance,
            default_name(db, object_type, instance),
            &initial_properties,
        )?
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
/// decoded whole by the property's value type on the object, the Object_Name kept
/// unique, and a NULL the property leaves as it is taken as applied
/// ([`relinquish`](bacnet_device_core::write::relinquish)). A whole value for one of the type's
/// creation-only properties goes to its initializer
/// ([`CreationOnly::initialize`]) instead of the write route, which keeps
/// refusing it (#1429). A State_Text written whole goes the write route, which sets
/// the state count from it (#1443); when the request gives the count
/// itself ([`StateCounts::Given`]), one that would change it is
/// VALUE_OUT_OF_RANGE. The caller removes the object when this fails.
fn initialize(
    db: &mut ObjectDatabase,
    creating: &Creating<'_>,
    pv: &bacnet_services::common::BACnetPropertyValue,
) -> Result<(), CreateObjectRefusal> {
    use bacnet_device_core::write::{
        WriteTarget, check_and_prepare_name_write, commit_attempt, decode_write_property_value,
    };
    use bacnet_objects::property_metadata::{check_write_array_index, object_value_type};
    let created_oid = creating.oid;
    let property = pv.property_identifier;
    let array_index = pv.property_array_index;
    let object = db.get(&created_oid).expect("created above");
    check_write_array_index(object, property, array_index).map_err(CreateObjectRefusal::Failed)?;
    // Octets that don't decode for the property make the request invalid,
    // not a write that was tried and failed.
    let value = decode_write_property_value(
        property,
        object_value_type(object, property),
        array_index,
        &pv.value,
    )
    .map_err(CreateObjectRefusal::Malformed)?;
    let value = bacnet_device_core::write::localize(db, created_oid, property, value);
    // The object was added under its default name; a new one has to be free,
    // and the database's name index moves along with it, whichever route
    // sets it.
    if property == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &created_oid, &value)
            .map_err(CreateObjectRefusal::Failed)?;
    }
    if array_index.is_none() && creating.creation_only.contains(property) {
        let applied = match creating
            .creation_only
            .initialize(db, &created_oid, property, value)
        {
            Ok(()) => true,
            // A NULL is judged as on the write route.
            Err(error)
                if bacnet_device_core::write::relinquish::is_null_octets(&pv.value)
                    && bacnet_device_core::write::relinquish::leaves_unchanged(
                        db.get(&created_oid).expect("created above"),
                        property,
                        None,
                        &error,
                    ) =>
            {
                false
            }
            Err(error) => return Err(CreateObjectRefusal::Failed(error)),
        };
        if applied && property == PropertyIdentifier::OBJECT_NAME {
            db.update_name_index(&created_oid);
        }
        return Ok(());
    }
    let target = WriteTarget {
        oid: created_oid,
        property,
        array_index,
        priority: pv.priority,
        value: &pv.value,
    };
    let fixed_count = (creating.counts == StateCounts::Given
        && property == PropertyIdentifier::STATE_TEXT
        && array_index.is_none())
    .then(|| state_count(db, created_oid));
    commit_attempt(db, None, target, value, None, creating.command_origin)
        .map_err(CreateObjectRefusal::Failed)?;
    // The labels have to fit the count the request gives; the caller drops
    // the object on a refusal, so the resize needs no undoing.
    match fixed_count {
        Some(count) if state_count(db, created_oid) != count => Err(CreateObjectRefusal::Failed(
            Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE),
        )),
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
/// Removes the object from the database. An absent object is OBJECT /
/// UNKNOWN_OBJECT whatever its type. One of a type DeleteObject doesn't
/// remove ([`bacnet_objects::creation::builtin_deleteable`]: the Device and
/// Network Port objects, which model the running node itself), or one the
/// database keeps for a running runtime
/// ([`ObjectDatabase::check_removable`]), is OBJECT /
/// OBJECT_DELETION_NOT_PERMITTED; the PICS reports the same type-level
/// answer. The running server also keeps the types its
/// [`ObjectTypeRegistry`](crate::server::ObjectTypeRegistry) keep from
/// deletion.
///
/// Returns the removed object. Drop it after releasing the database guard,
/// and off the async runtime: an object that saves its state (a Notification
/// Forwarder or Audit Log with persistence) waits for its queued saves when
/// dropped (see [`bacnet_objects::durable`]).
pub fn handle_delete_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Box<dyn bacnet_objects::traits::BACnetObject>, Error> {
    let constructors = crate::server::ObjectTypeRegistry::default();
    delete_object(db, &constructors, service_data)
}

/// [`handle_delete_object`] with the server's registry.
pub(crate) fn delete_object(
    db: &mut ObjectDatabase,
    constructors: &crate::server::ObjectTypeRegistry,
    service_data: &[u8],
) -> Result<Box<dyn bacnet_objects::traits::BACnetObject>, Error> {
    let request = DeleteObjectRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let oid = request.object_identifier;
    if db.get(&oid).is_none() {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ));
    }
    if !constructors.deletes(oid.object_type()) {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_DELETION_NOT_PERMITTED,
        ));
    }
    db.remove(&oid)?.ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))
}
