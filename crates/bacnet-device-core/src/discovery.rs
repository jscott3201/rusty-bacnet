//! Who-Is and I-Am (Clause 16.10), Who-Has and I-Have (Clause 16.9), against
//! a borrowed database. The host keeps the rate limits and chooses between a
//! directed and a broadcast answer.
use alloc::string::ToString;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::IAmRequest;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// Handle a WhoHas request and return an IHave response if we have the object.
///
/// Returns `Some(IHaveRequest)` if we have the requested object, `None` otherwise.
pub fn who_has(
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

/// I-Am from the server's frozen, resolved Device declarations.
pub fn i_am(identity: &DeviceIdentity) -> IAmRequest {
    IAmRequest {
        object_identifier: identity.device_oid(),
        max_apdu_length: u32::from(identity.max_apdu_length()),
        segmentation_supported: identity.segmentation(),
        vendor_id: identity.vendor_id(),
    }
}

/// Membership may change, but announcement identity never rebinds. Served
/// identity properties are projected from the same frozen declarations.
pub fn announcement_device(
    db: &ObjectDatabase,
    identity: &DeviceIdentity,
) -> Option<ObjectIdentifier> {
    let oid = identity.device_oid();
    db.get(&oid)
        .filter(|object| object.object_identifier() == oid)
        .map(|_| oid)
}
