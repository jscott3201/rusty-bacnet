use super::*;
use bacnet_device_core::write::{self, WpmOutcome};

/// The handler tests' WritePropertyMultiple: bacnet-device-core's WPM with
/// no gate, observer, source or command origin. The complete successful
/// prefix remains committed if a later attempt fails. A Command or Channel
/// object's Present_Value write in that prefix runs nothing: as in
/// `Responder::write`, its list or distribution ends at once as
/// unsuccessful. Running servers make those runs instead.
pub(crate) fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    match write::write_property_multiple(db, service_data, None, None, None, None) {
        WpmOutcome::Success { committed_oids } => {
            write::end_unmade(db, &committed_oids);
            Ok(committed_oids)
        }
        WpmOutcome::Error {
            error,
            committed_oids,
            ..
        } => {
            write::end_unmade(db, &committed_oids);
            Err(error)
        }
        WpmOutcome::Reject { reason } => Err(Error::reject(reason)),
    }
}

/// The handler tests' WriteProperty: bacnet-device-core's write with no
/// observer, source or command origin, ending any Command or Channel run the
/// write queues at once, unmade, as `Responder::write` does. Running servers
/// make those runs instead.
pub(crate) fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let (oid, _) = write::write_property(db, service_data, None, None, None)?;
    write::end_unmade(db, std::slice::from_ref(&oid));
    Ok(oid)
}
