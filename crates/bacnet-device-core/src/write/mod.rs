//! The write services against a borrowed [`ObjectDatabase`]: WriteProperty,
//! the checks and decoding every write path shares, the NULL-relinquish
//! rule, the local form of references naming this device, and the end of a
//! Command or Channel run a write queued that its host can't make.
//!
//! The host owns the database guard. A [`WriteCommitObserver`] runs under
//! it, synchronously; [its documentation](WriteCommitObserver) says which
//! locks an implementation may take.
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::AuditWriteSource;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

mod decode;
mod executor;
mod local_references;
pub mod relinquish;
mod runs;

pub use decode::{check_write_array_index, decode_write_property_value, gate_and_decode_write};
pub use executor::{check_executor_owned_write, identity_property, owned};
pub use local_references::{DeviceQualified, localize, localize_member};
pub use runs::{Unfinished, end_unmade, take_queued};

/// Borrowed write coordinates; never retained across the synchronous commit.
#[derive(Clone, Copy)]
pub struct WriteTarget<'a> {
    /// The object written.
    pub oid: ObjectIdentifier,
    /// The property written.
    pub property: PropertyIdentifier,
    /// The array index written, if any.
    pub array_index: Option<u32>,
    /// The write's priority, if any.
    pub priority: Option<u8>,
    /// The value's octets, as the request carried them.
    pub value: &'a [u8],
}

/// What a host does around each write attempt, under its database guard.
///
/// Every call is synchronous and runs while the host holds the database
/// for writing. An implementation must not await, and must take no lock
/// that can wait on the database, on an async task, or on anything held
/// across an await. A short leaf mutex, taken after the database and held
/// for the call alone, is fine: the server's COV capture takes its timed
/// store's, and its Audit Reporter its delivery bookkeeping's.
pub trait WriteCommitObserver: Send {
    /// The attempt on `write` is about to run, after its checks.
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
pub enum Applied {
    /// The object took the value: the post-write work for a change follows.
    Written,
    /// A NULL the property left as it was ([`relinquish`]):
    /// nothing changed, so no post-write work follows.
    Unchanged,
}

/// Validate database-owned Object_Name uniqueness before mutation.
pub fn check_and_prepare_name_write(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    value: &PropertyValue,
) -> Result<(), Error> {
    if let PropertyValue::CharacterString(new_name) = value {
        db.check_name_available(oid, new_name)?;
    }
    Ok(())
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
pub fn commit_attempt(
    db: &mut ObjectDatabase,
    mut observer: Option<&mut (dyn WriteCommitObserver + '_)>,
    target: WriteTarget<'_>,
    value: PropertyValue,
    source: Option<&AuditWriteSource>,
    command_origin: Option<&CommandOrigin>,
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
            if relinquish::is_null_octets(target.value)
                && relinquish::leaves_unchanged(
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

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::protocol(class, code)
}

/// Handle a WriteProperty for the server: the object written, and whether
/// the write changed it or was a NULL the property left as it was.
pub fn write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&AuditWriteSource>,
    command_origin: Option<&CommandOrigin>,
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
    let value = localize(db, oid, request.property_identifier, value);
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &oid, &value)?;
    }
    if request.property_identifier == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
        && let Some(mut device) = db
            .get_mut(&oid)
            .and_then(|object| object.device_authority_internal())
        && device.object_identifier() == oid
    {
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
    object: &mut dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    source: Option<&AuditWriteSource>,
    command_origin: Option<&CommandOrigin>,
) -> Result<(), Error> {
    check_executor_owned_write(object.object_identifier(), property)?;
    if matches!(
        property,
        PropertyIdentifier::DESCRIPTION
            | PropertyIdentifier::MAXIMUM_SEND_DELAY
            | PropertyIdentifier::SEND_NOW
    ) && let Some(mut reporter) = object.audit_reporter_authority_internal()
    {
        return reporter.write_property(property, value, index, source);
    }
    match command_origin {
        Some(origin) => object.write_property_from(property, index, value, priority, origin),
        None => object.write_property(property, index, value, priority),
    }
}
