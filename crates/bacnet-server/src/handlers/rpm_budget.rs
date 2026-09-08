//! Bounded server-owned RPM planning and service-ACK accumulation.
use super::*;
use crate::server::ReadPropertyMultipleBudget;
use bacnet_objects::{property_metadata::PropertyConformance, traits::BACnetObject};
use bacnet_services::common::PropertyReference;

#[derive(Debug)]
pub(crate) enum RpmFailure {
    Service(Error),
    Work,
    Bytes,
}

struct PlannedObject {
    response_oid: ObjectIdentifier,
    lookup_oid: ObjectIdentifier,
    properties: Vec<PropertyReference>,
}

// Visit expansion rows without collecting a second, unbounded expansion vector.
// Opaque metadata/property-list allocation is outside this service budget.
fn expand(
    object: &dyn BACnetObject,
    reference: &PropertyReference,
    mut visit: impl FnMut(PropertyIdentifier) -> Result<(), RpmFailure>,
) -> Result<(), RpmFailure> {
    let id = reference.property_identifier;
    if !matches!(
        id,
        PropertyIdentifier::ALL | PropertyIdentifier::REQUIRED | PropertyIdentifier::OPTIONAL
    ) {
        return visit(id);
    }
    let metadata = object.property_metadata();
    if !metadata.is_empty() {
        for row in metadata.iter() {
            let selected = match id {
                PropertyIdentifier::ALL => {
                    row.property_identifier != PropertyIdentifier::PROPERTY_LIST
                }
                PropertyIdentifier::REQUIRED => {
                    row.property_identifier != PropertyIdentifier::PROPERTY_LIST
                        && row.conformance.is_required()
                }
                _ => row.conformance == PropertyConformance::Optional,
            };
            if selected {
                visit(row.property_identifier)?;
            }
        }
    } else {
        match id {
            PropertyIdentifier::ALL => {
                for &property in object.property_list().iter() {
                    visit(property)?;
                }
            }
            PropertyIdentifier::REQUIRED => {
                for &property in object.required_properties().iter() {
                    visit(property)?;
                }
            }
            _ => {
                // Preserve legacy expected-linear selection even when every
                // row is required and the result budget never cuts the scan.
                let required: HashSet<PropertyIdentifier> =
                    object.required_properties().iter().copied().collect();
                for &property in object.property_list().iter() {
                    if !required.contains(&property) {
                        visit(property)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn plan(
    db: &ObjectDatabase,
    request: &ReadPropertyMultipleRequest,
    limit: usize,
) -> Result<Vec<PlannedObject>, RpmFailure> {
    let mut plan = Vec::new();
    let mut count = 0usize;
    for spec in &request.list_of_read_access_specs {
        let lookup_oid = read_property::resolve_device_wildcard(db, &spec.object_identifier);
        let mut properties = Vec::new();
        for reference in &spec.list_of_property_references {
            let mut push = |id| {
                count = count
                    .checked_add(1)
                    .filter(|&n| n <= limit)
                    .ok_or(RpmFailure::Work)?;
                properties.push(PropertyReference {
                    property_identifier: id,
                    property_array_index: if id == reference.property_identifier {
                        reference.property_array_index
                    } else {
                        None
                    },
                });
                Ok(())
            };
            match db.get(&lookup_oid) {
                Some(object) => expand(object, reference, push)?,
                None => push(reference.property_identifier)?,
            }
        }
        // Empty object wrappers are retained; their count is bounded by the
        // existing decoded request, not by the expanded-result policy.
        plan.push(PlannedObject {
            response_oid: spec.object_identifier,
            lookup_oid,
            properties,
        });
    }
    Ok(plan)
}

fn element(object: Option<&dyn BACnetObject>, reference: &PropertyReference) -> ReadResultElement {
    let id = reference.property_identifier;
    let index = reference.property_array_index;
    let result = match object {
        None => Err((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
        Some(object) if index.is_some() && !object.is_array_property(id) => {
            Err((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
        }
        Some(object) => match object.read_property(id, index) {
            Ok(value) => {
                // One property may return/encode an arbitrarily large owned
                // value. Only accumulated service bytes are bounded here.
                let mut encoded = BytesMut::new();
                encode_property_value(&mut encoded, &value)
                    .map(|()| encoded.to_vec())
                    .map_err(|_| (ErrorClass::PROPERTY, ErrorCode::OTHER))
            }
            Err(Error::Protocol { class, code }) => Err((
                ErrorClass::from_raw(class as u16),
                ErrorCode::from_raw(code as u16),
            )),
            Err(_) => Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        },
    };
    let (property_value, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    ReadResultElement {
        property_identifier: id,
        property_array_index: index,
        property_value,
        error,
    }
}

struct Scratch {
    bytes: BytesMut,
    limit: usize,
}

impl Scratch {
    fn append(&mut self, bytes: &[u8], reserved: usize) -> Result<(), RpmFailure> {
        self.bytes
            .len()
            .checked_add(bytes.len())
            .and_then(|n| n.checked_add(reserved))
            .filter(|&n| n <= self.limit)
            .ok_or(RpmFailure::Bytes)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

/// Atomic with respect to the caller's buffer, not object read side effects.
pub(crate) fn handle_rpm_budgeted(
    db: &ObjectDatabase,
    data: &[u8],
    buf: &mut BytesMut,
    budget: ReadPropertyMultipleBudget,
) -> Result<(), RpmFailure> {
    let request = ReadPropertyMultipleRequest::decode(data).map_err(RpmFailure::Service)?;
    let plan = plan(db, &request, budget.max_result_elements)?;
    let mut scratch = Scratch {
        bytes: BytesMut::new(),
        limit: budget.max_service_ack_bytes,
    };
    let mut footer = BytesMut::new();
    ReadAccessResult::encode_footer(&mut footer);
    for spec in plan {
        let mut header = BytesMut::new();
        ReadAccessResult::encode_header(&mut header, &spec.response_oid);
        scratch.append(&header, footer.len())?;
        for reference in spec.properties {
            let result = element(db.get(&spec.lookup_oid), &reference);
            let mut encoded = BytesMut::new();
            result.encode(&mut encoded);
            scratch.append(&encoded, footer.len())?;
        }
        scratch.append(&footer, 0)?;
    }
    buf.extend_from_slice(&scratch.bytes);
    Ok(())
}

#[cfg(test)]
#[path = "tests/rpm_budget.rs"]
mod tests;
