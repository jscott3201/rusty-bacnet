use super::*;

/// Borrowed write coordinates; never retained across the synchronous commit.
#[derive(Clone, Copy)]
pub(crate) struct WriteTarget<'a> {
    pub oid: ObjectIdentifier,
    pub property: PropertyIdentifier,
    pub array_index: Option<u32>,
    pub priority: Option<u8>,
    pub value: &'a [u8],
}

pub(crate) trait WriteCommitObserver: Send {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>);
    /// Handle an eligible sealed policy assignment through atomic Audit admission.
    /// None keeps the ordinary object writer; Some owns the complete result.
    fn commit_policy(
        &mut self,
        _db: &mut ObjectDatabase,
        _write: WriteTarget<'_>,
        _value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        None
    }
    /// The attempt after `before` succeeded. A NULL the property left as it
    /// was ([`relinquish`]) succeeds too, and gets this
    /// call without [`applied`](Self::applied).
    fn committed(&mut self, db: &mut ObjectDatabase);
    /// Execution returned an error after `before`; never called for authorization denial.
    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error);
    /// A write attempt on `oid` succeeded and its effects are in `db`. Called
    /// after `committed`, and also for the Device-owned recipient write that
    /// bypasses `before` and `committed`.
    fn applied(&mut self, _db: &ObjectDatabase, _oid: ObjectIdentifier) {}
}

/// What a successful write attempt did to its object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The object took the value: the post-write work for a change follows.
    Written,
    /// A NULL the property left as it was ([`relinquish`]):
    /// nothing changed, so no post-write work follows.
    Unchanged,
}

/// Validate database-owned Object_Name uniqueness before mutation.
pub(super) fn check_and_prepare_name_write(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    value: &PropertyValue,
) -> Result<(), Error> {
    if let PropertyValue::CharacterString(new_name) = value {
        db.check_name_available(oid, new_name)?;
    }
    Ok(())
}

/// Rich WPM result retained inside the server boundary.
pub(crate) enum WritePropertyMultipleOutcome {
    Success {
        committed_oids: Vec<ObjectIdentifier>,
    },
    Error {
        error: Error,
        first_failed_write_attempt: BACnetObjectPropertyReference,
        committed_oids: Vec<ObjectIdentifier>,
    },
    Reject {
        reason: RejectReason,
    },
}

/// Handle WPM while preserving the historical direct handler projection.
///
/// The complete successful prefix remains committed if a later attempt fails.
/// A Command or Channel object's Present_Value write in that prefix runs
/// nothing: as in [`handle_write_property`], its list or distribution ends at
/// once as unsuccessful.
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    match handle_write_property_multiple_detailed(db, service_data, &mut snapshots) {
        WritePropertyMultipleOutcome::Success { committed_oids } => {
            crate::command_lists::end_unmade(db, &committed_oids);
            Ok(committed_oids)
        }
        WritePropertyMultipleOutcome::Error {
            error,
            committed_oids,
            ..
        } => {
            crate::command_lists::end_unmade(db, &committed_oids);
            Err(error)
        }
        WritePropertyMultipleOutcome::Reject { reason } => Err(Error::Reject {
            reason: reason.to_raw(),
        }),
    }
}

/// Execute WPM incrementally in wire order for server dispatch.
pub(crate) fn handle_write_property_multiple_detailed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
) -> WritePropertyMultipleOutcome {
    handle_write_property_multiple_authorized(db, service_data, snapshots, None)
}

type WritePropertyMultipleGate<'a> =
    &'a dyn Fn(&bacnet_services::wpm::WritePropertyAttempt) -> Result<(), Error>;

/// Optional per-element gate; `None` preserves the existing incremental path.
pub(crate) fn handle_write_property_multiple_authorized(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
    authorize: Option<WritePropertyMultipleGate<'_>>,
) -> WritePropertyMultipleOutcome {
    handle_write_property_multiple_observed(
        db,
        service_data,
        snapshots,
        authorize,
        None,
        None,
        None,
    )
}

pub(crate) fn handle_write_property_multiple_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
    authorize: Option<WritePropertyMultipleGate<'_>>,
    mut observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> WritePropertyMultipleOutcome {
    let mut cursor = WritePropertyMultipleCursor::new(service_data);
    let mut committed_oids = Vec::new();
    // Whether any attempt has succeeded. A NULL the property left as it was
    // commits no object but is a successful write all the same, so a later
    // syntax error is an Error with INVALID_TAG, not a Reject (Clause
    // 15.10.2).
    let mut wrote = false;

    loop {
        let event = match cursor.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                return WritePropertyMultipleOutcome::Success { committed_oids };
            }
            Err(cursor_error) => {
                use bacnet_services::wpm::WritePropertyMultipleFailureKind;
                let code = match cursor_error.kind {
                    WritePropertyMultipleFailureKind::PriorityOutOfRange => {
                        ErrorCode::PARAMETER_OUT_OF_RANGE
                    }
                    WritePropertyMultipleFailureKind::Syntax(reason) if !wrote => {
                        return WritePropertyMultipleOutcome::Reject { reason };
                    }
                    WritePropertyMultipleFailureKind::Syntax(_) => ErrorCode::INVALID_TAG,
                };
                return WritePropertyMultipleOutcome::Error {
                    error: protocol_error(ErrorClass::SERVICES, code),
                    first_failed_write_attempt: cursor_error
                        .first_failed_write_attempt
                        .unwrap_or_else(wpm_undecodable_coordinate),
                    committed_oids,
                };
            }
        };
        let WritePropertyMultipleEvent::WriteAttempt(attempt) = event else {
            continue;
        };
        let reference = attempt.reference.clone();
        let oid = reference.object_identifier;
        let property = PropertyIdentifier::from_raw(reference.property_identifier);

        let Some(object) = db.get(&oid) else {
            return semantic_failure(
                protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                reference,
                committed_oids,
            );
        };
        let value = match gate_and_decode_write(
            object,
            property,
            reference.property_array_index,
            &attempt.value,
        ) {
            Ok(value) => crate::local_references::localize(db, oid, property, value),
            Err(error) => return semantic_failure(error, reference, committed_oids),
        };
        if property == PropertyIdentifier::OBJECT_NAME {
            if let Err(error) = check_and_prepare_name_write(db, &oid, &value) {
                return semantic_failure(error, reference, committed_oids);
            }
        }

        if let Some(authorize) = authorize {
            if let Err(error) = authorize(&attempt) {
                return semantic_failure(error, reference, committed_oids);
            }
        }
        if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
            if let Some(mut device) = db
                .get_mut(&oid)
                .and_then(|object| object.device_authority_internal())
            {
                if device.object_identifier() == oid {
                    if let Err(error) = device.write_audit_recipient(
                        reference.property_array_index,
                        value,
                        attempt.priority,
                        source,
                    ) {
                        return semantic_failure(error, reference, committed_oids);
                    }
                    if let Some(observer) = observer.as_deref_mut() {
                        observer.applied(db, oid);
                    }
                    wrote = true;
                    if !committed_oids.contains(&oid) {
                        committed_oids.push(oid);
                    }
                    continue;
                }
            }
        }
        snapshots.capture_before_write(db, oid);
        let target = WriteTarget {
            oid,
            property,
            array_index: reference.property_array_index,
            priority: attempt.priority,
            value: &attempt.value,
        };
        match commit_attempt(
            db,
            observer.as_deref_mut(),
            target,
            value,
            source,
            command_origin,
        ) {
            // A NULL the property left as it was commits nothing.
            Ok(Applied::Unchanged) => wrote = true,
            Ok(Applied::Written) => {
                wrote = true;
                if !committed_oids.contains(&oid) {
                    committed_oids.push(oid);
                }
            }
            Err(error) => return semantic_failure(error, reference, committed_oids),
        }
    }
}

/// Make one checked write attempt: the object's write, or the observer's
/// sealed policy commit, between the observer's `before` and its
/// `committed` or `failed`.
///
/// A NULL the object refuses as the wrong datatype succeeds unchanged when
/// [`relinquish::leaves_unchanged`]
/// says so: the observer gets `committed`, so an Audit Reporter records the
/// successful write, but not `applied`, since there is no change to capture.
/// CreateObject applies each initial value through here too, with no observer.
pub(super) fn commit_attempt(
    db: &mut ObjectDatabase,
    mut observer: Option<&mut (dyn WriteCommitObserver + '_)>,
    target: WriteTarget<'_>,
    value: PropertyValue,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Result<Applied, Error> {
    if let Some(observer) = observer.as_deref_mut() {
        observer.before(db, target);
    }
    let prepared = observer
        .as_deref_mut()
        .and_then(|observer| observer.commit_policy(db, target, &value));
    let result = prepared.unwrap_or_else(|| {
        write_with_source(
            db.get_mut(&target.oid).expect("existence checked above"),
            target.property,
            target.array_index,
            value,
            target.priority,
            source,
            command_origin,
        )
    });
    let applied = match result {
        Ok(()) => Applied::Written,
        Err(error)
            if super::relinquish::is_null_octets(target.value)
                && super::relinquish::leaves_unchanged(
                    db.get(&target.oid).expect("existence checked above"),
                    target.property,
                    target.array_index,
                    &error,
                ) =>
        {
            Applied::Unchanged
        }
        Err(error) => {
            if let Some(observer) = observer {
                observer.failed(db, &error);
            }
            return Err(error);
        }
    };
    if applied == Applied::Written && target.property == PropertyIdentifier::OBJECT_NAME {
        db.update_name_index(&target.oid);
    }
    // A Pulse Converter judges its new Input_Reference against the database
    // as a WriteProperty or WritePropertyMultiple commits it, so Reliability
    // reads right at once (#1341). The written object's own COV pass carries
    // the change. (CreateObject's initial values come through here too, but
    // CreateObject builds no Pulse Converter.)
    if applied == Applied::Written && target.property == PropertyIdentifier::INPUT_REFERENCE {
        db.check_input_reference(&target.oid);
    }
    if let Some(observer) = observer {
        observer.committed(db);
        if applied == Applied::Written {
            observer.applied(db, target.oid);
        }
    }
    Ok(applied)
}

fn semantic_failure(
    error: Error,
    first_failed_write_attempt: BACnetObjectPropertyReference,
    committed_oids: Vec<ObjectIdentifier>,
) -> WritePropertyMultipleOutcome {
    WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    }
}

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

/// At the existing indexed-write gate, effective absence takes precedence over
/// array classification (local error-order policy for Clauses 15.9/15.10).
pub(crate) fn check_write_array_index(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<(), Error> {
    if index.is_none() {
        return Ok(());
    }
    let metadata = object.property_metadata();
    // Empty metadata is the optional custom/unmigrated default, not proof of
    // absence. Preserve those objects' classifier and write dispatch.
    if !metadata.is_empty()
        && !metadata
            .iter()
            .any(|row| row.property_identifier == property)
    {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ));
    }
    if !object.is_array_property(property) {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }
    Ok(())
}

/// What WriteProperty does with a value's octets before `object` takes it:
/// [`check_write_array_index`], then [`decode_write_property_value`] with the
/// object's list classification. WriteProperty, WritePropertyMultiple,
/// `write_local_encoded` and a Command's or Channel's local writes share it,
/// so each answers an index the same way and ahead of the value's decoding.
/// The typed local paths, which have no octets, run the index check alone.
pub(crate) fn gate_and_decode_write(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    check_write_array_index(object, property, array_index)?;
    decode_write_property_value(
        property,
        array_index,
        object.is_list_property(property),
        bytes,
    )
}

fn wpm_undecodable_coordinate() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference {
        // Clause 15.10 fixes instance 4194303. DEVICE / ALL / no index is the
        // repository's local policy for the remaining undecodable coordinates.
        object_identifier: ObjectIdentifier::new(
            ObjectType::DEVICE,
            ObjectIdentifier::MAX_INSTANCE,
        )
        .expect("the wildcard instance is valid service vocabulary"),
        property_identifier: PropertyIdentifier::ALL.to_raw(),
        property_array_index: None,
    }
}

/// PROPERTY / INVALID_DATA_ENCODING for an undecodable propertyValue payload.
fn invalid_data_encoding_error() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
}

/// Decode the complete propertyValue payload handed to an object write arm.
///
/// A few properties come first and reach their object as it decodes them
/// itself: raw octets in `PropertyValue::ApplicationData` (Recipient_List,
/// Subscribed_Recipients, List_Of_Object_Property_References and the other
/// cases below), or one chunk per array element. Each such arm says what an
/// empty value means; a Setpoint_Reference with no octets, for one, holds no
/// reference.
///
/// Everything else goes through the generic loop. There, `list` says
/// whether the target object holds `property` as a BACnetLIST
/// ([`BACnetObject::is_list_property`]). Such a property written whole
/// reaches the object as a `PropertyValue::List` whatever its length, so an
/// empty value clears it and one element arrives as a list of one rather
/// than as that element alone. For a property that isn't a list, an empty
/// value is INVALID_DATA_ENCODING.
///
/// [`BACnetObject::is_list_property`]: bacnet_objects::traits::BACnetObject::is_list_property
pub(crate) fn decode_write_property_value(
    property: PropertyIdentifier,
    array_index: Option<u32>,
    list: bool,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    if property == PropertyIdentifier::EVENT_PARAMETERS && bytes.starts_with(&[0xfe, 0xff]) {
        use bacnet_types::constructed::BACnetEventParameter;

        return match bacnet_encoding::constructed::decode_event_parameter(bytes, 0) {
            Ok((BACnetEventParameter::Opaque { tag, data }, consumed))
                if tag == u8::MAX && consumed == bytes.len() =>
            {
                Ok(PropertyValue::OctetString(data))
            }
            _ => Err(invalid_data_encoding_error()),
        };
    }
    // The Schedule decodes its list of references itself, an empty list
    // included (#1088), once the handler has put members naming this device
    // in their local form (#1122). So does a Channel, whose references are an
    // array: index 0, the array size, stays an Unsigned (#1151). A Lighting
    // Output decodes its Lighting_Command whole too, so a command keeps its
    // fields together and any other datatype is the object's to refuse
    // (#1263), and so do Color and Color Temperature their Color_Command
    // (#1386).
    if matches!(
        property,
        PropertyIdentifier::RECIPIENT_LIST
            | PropertyIdentifier::SUBSCRIBED_RECIPIENTS
            | PropertyIdentifier::VALUE_SOURCE
            | PropertyIdentifier::EFFECTIVE_PERIOD
            | PropertyIdentifier::LIGHTING_COMMAND
            | PropertyIdentifier::COLOR_COMMAND
    ) || (property == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
        && array_index != Some(0))
    {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // The Schedule's arrays of constructed elements, an Access Rights
    // object's two rule arrays, and Tags, reach the object as raw bytes,
    // which it splits and decodes with the shared codecs; index 0, the array
    // size, stays an Unsigned (#1057, #1330, #1553).
    if array_index != Some(0)
        && matches!(
            property,
            PropertyIdentifier::WEEKLY_SCHEDULE
                | PropertyIdentifier::EXCEPTION_SCHEDULE
                | PropertyIdentifier::POSITIVE_ACCESS_RULES
                | PropertyIdentifier::NEGATIVE_ACCESS_RULES
                | PropertyIdentifier::TAGS
        )
    {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
        use bacnet_types::constructed::BACnetRecipient;

        // Clause 15.9/15.10 relinquishment is an operation, not a Recipient
        // value or disabled sentinel. The object owner must leave state intact.
        if let Ok((tag, end)) = bacnet_encoding::tags::decode_tag(bytes, 0) {
            if tag.class == bacnet_encoding::tags::TagClass::Application
                && tag.number == bacnet_encoding::tags::app_tag::NULL
                && tag.length == 0
                && end == bytes.len()
            {
                return Ok(PropertyValue::Null);
            }
        }
        // An address MAC past the bound does not decode, so it is refused
        // with the other undecodable values (#1124).
        let (recipient, consumed) = bacnet_encoding::constructed::decode_recipient(bytes, 0)
            .map_err(|_| invalid_data_encoding_error())?;
        if consumed != bytes.len() {
            return Err(invalid_data_encoding_error());
        }
        if let BACnetRecipient::Device(oid) = recipient {
            if oid.object_type() != ObjectType::DEVICE
                || oid.instance_number() == ObjectIdentifier::MAX_INSTANCE
            {
                return Err(invalid_data_encoding_error());
            }
        }
        // This validates the single Recipient value, not its deliverability.
        // Address routing and broadcast policy belong to the mutation owner.
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // Log_DeviceObjectProperty and an Averaging object's
    // Object_Property_Reference reach the object as raw reference bytes, which
    // it decodes with the shared device-reference helpers, after the handler
    // has put any reference naming this device in its local form; a Trend Log
    // Multiple's index 0, the array size, stays an Unsigned (#1234, #1313).
    // An application Null alone reaches it as Null, which it refuses as the
    // wrong datatype, so the write is judged as a relinquish (`relinquish`,
    // #1417).
    if array_index != Some(0)
        && matches!(
            property,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY
                | PropertyIdentifier::OBJECT_PROPERTY_REFERENCE
        )
    {
        if bytes == [0x00] {
            return Ok(PropertyValue::Null);
        }
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // The Loop and Pulse Converter references reach the object as their raw
    // octets too, which it decodes with the shared codecs: an empty value is
    // a Setpoint_Reference holding no reference (#1312). An application Null
    // alone reaches the object as Null, judged as above (#1417).
    if matches!(
        property,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE
            | PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE
            | PropertyIdentifier::SETPOINT_REFERENCE
            | PropertyIdentifier::INPUT_REFERENCE
    ) {
        if bytes == [0x00] {
            return Ok(PropertyValue::Null);
        }
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // A Notification Forwarder's Port_Filter goes one BACnetPortPermission
    // per chunk (#1225).
    if array_index != Some(0) && property == PropertyIdentifier::PORT_FILTER {
        return decode_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_port_permission(data, offset).map(|(_, end)| end)
        });
    }
    if array_index != Some(0) && property == PropertyIdentifier::STAGES {
        return decode_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_stage_limit_value(data, offset).map(|(_, end)| end)
        });
    }
    // Staging targets reach the object as raw reference bytes too, decoded
    // by the same helpers, once the handler has put those naming this device
    // in their local form (#1136, #1313); index 0, the array size, stays an
    // Unsigned.
    if array_index != Some(0) && property == PropertyIdentifier::TARGET_REFERENCES {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(bytes, offset)
                .map_err(|_| invalid_data_encoding_error())?;
        values.push(value);
        offset = new_offset;
    }
    // A list's encoding is its elements back to back, nothing at all for an
    // empty one (Clause 20.2.17). The other datatypes reaching this loop have
    // no empty encoding, so no octets is INVALID_DATA_ENCODING for them
    // (Clause 15.9.1.3).
    if list && array_index.is_none() {
        return Ok(PropertyValue::List(values));
    }
    match values.len() {
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

fn decode_structured_array<F>(
    bytes: &[u8],
    array_index: Option<u32>,
    mut decode: F,
) -> Result<PropertyValue, Error>
where
    F: FnMut(&[u8], usize) -> Result<usize, Error>,
{
    // A lone NULL is no element of these arrays. It reaches the object as
    // Null, which refuses it as the wrong datatype, so the write is judged
    // as a relinquish (`relinquish`) rather than as undecodable octets.
    if super::relinquish::is_null_octets(bytes) {
        return Ok(PropertyValue::Null);
    }
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if values.len() >= 10_000 {
            return Err(invalid_data_encoding_error());
        }
        let start = offset;
        offset = decode(bytes, offset).map_err(|_| invalid_data_encoding_error())?;
        if offset <= start || offset > bytes.len() {
            return Err(invalid_data_encoding_error());
        }
        values.push(PropertyValue::ApplicationData(
            bytes[start..offset].to_vec(),
        ));
    }
    if array_index.is_some() {
        if values.len() == 1 {
            return Ok(values.pop().unwrap());
        }
        return Err(invalid_data_encoding_error());
    }
    Ok(PropertyValue::List(values))
}

/// Handle a WriteProperty request.
///
/// This synchronous handler makes no writes on a Command or Channel object's
/// behalf: it has no task to wait out a delay in. A Present_Value write that
/// selects a list with commands is accepted, and the run it starts ends at
/// once with every command unsuccessful, so In_Process is FALSE again on
/// return and All_Writes_Successful is FALSE (#1178). A Channel's
/// Present_Value write is accepted too, and the distribution it starts ends at
/// once without writing the members, so Write_Status reads FAILED on return
/// (#1151). The bundled [`BACnetServer`](crate::server::BACnetServer) runs
/// both.
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let (oid, _) = handle_write_property_observed(db, service_data, None, None, None)?;
    crate::command_lists::end_unmade(db, std::slice::from_ref(&oid));
    Ok(oid)
}

/// Handle a WriteProperty for the server: the object written, and whether
/// the write changed it or was a NULL the property left as it was.
pub(crate) fn handle_write_property_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Result<(ObjectIdentifier, Applied), Error> {
    let request = WritePropertyRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let oid = request.object_identifier;

    let object = db
        .get(&oid)
        .ok_or_else(|| protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))?;
    let value = gate_and_decode_write(
        object,
        request.property_identifier,
        request.property_array_index,
        &request.property_value,
    )?;
    let value = crate::local_references::localize(db, oid, request.property_identifier, value);
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &oid, &value)?;
    }
    if request.property_identifier == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
        if let Some(mut device) = db
            .get_mut(&oid)
            .and_then(|object| object.device_authority_internal())
        {
            if device.object_identifier() == oid {
                device.write_audit_recipient(
                    request.property_array_index,
                    value,
                    request.priority,
                    source,
                )?;
                if let Some(observer) = observer {
                    observer.applied(db, oid);
                }
                return Ok((oid, Applied::Written));
            }
        }
    }
    let target = WriteTarget {
        oid,
        property: request.property_identifier,
        array_index: request.property_array_index,
        priority: request.priority,
        value: &request.property_value,
    };
    let applied = commit_attempt(db, observer, target, value, source, command_origin)?;
    Ok((oid, applied))
}

// Concrete Reporter changes keep the authorized request provenance at their
// canonical object-owned mutation boundary. All other object writes stay generic.
fn write_with_source(
    object: &mut dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> Result<(), Error> {
    crate::device_view::check_executor_owned_write(object.object_identifier(), property)?;
    if matches!(
        property,
        PropertyIdentifier::DESCRIPTION
            | PropertyIdentifier::MAXIMUM_SEND_DELAY
            | PropertyIdentifier::SEND_NOW
    ) {
        if let Some(mut reporter) = object.audit_reporter_authority_internal() {
            return reporter.write_property(property, value, index, source);
        }
    }
    match command_origin {
        Some(origin) => object.write_property_from(property, index, value, priority, origin),
        None => object.write_property(property, index, value, priority),
    }
}
