//! Bounded server-owned RPM planning and service-ACK accumulation.
use super::group_present_value::GroupMembers;
use super::read_budget::{ReadFailure, Work};
use super::*;
#[cfg(test)]
use crate::server::ReadPropertyMultipleBudget;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};

/// One specification after target resolution and ALL/REQUIRED/OPTIONAL
/// expansion; a Group's Present_Value plans its members the same way.
pub(super) struct PlannedObject {
    pub(super) lookup_oid: ObjectIdentifier,
    pub(super) properties: Vec<PlannedRow>,
}

/// One expanded result row. A row that reads a Group's whole Present_Value
/// also carries the Group's members, planned and charged with the row.
pub(super) struct PlannedRow {
    pub(super) reference: PropertyReference,
    pub(super) members: Option<GroupMembers>,
}

// Visit expansion rows without collecting a second, unbounded expansion vector.
// Opaque metadata/property-list allocation is outside this service budget.
fn expand(
    object: &dyn BACnetObject,
    reference: &PropertyReference,
    mut visit: impl FnMut(PropertyIdentifier) -> Result<(), ReadFailure>,
) -> Result<(), ReadFailure> {
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
                        && row.is_required()
                }
                _ => !row.is_required(),
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

/// Every planned row of `objects`, with the object it reads.
pub(super) fn planned_rows(
    objects: &[PlannedObject],
) -> impl Iterator<Item = (ObjectIdentifier, &PlannedRow)> {
    objects.iter().flat_map(|object| {
        object
            .properties
            .iter()
            .map(move |row| (object.lookup_oid, row))
    })
}

/// The selected Device's server-owned lists that planned `rows` read, its COV
/// lists and its Device_Address_Binding (#1369): a row naming one on that
/// Device, or such a row among the planned members of a Group whose whole
/// Present_Value is read (#1171). A plan holds exactly the rows the request
/// will read, ALL, REQUIRED and OPTIONAL already expanded, so the request
/// samples only the lists it serves (#1213).
pub(super) fn live_cov_selection<'a>(
    db: &ObjectDatabase,
    rows: impl IntoIterator<Item = (ObjectIdentifier, &'a PlannedRow)>,
) -> Option<LiveCovSelection> {
    // Only a plan that names one of the lists pays the Device lookup.
    let mut device = None;
    let mut selection: Option<LiveCovSelection> = None;
    let mut select = |oid: ObjectIdentifier, row: &PlannedRow| {
        let (active, multiple, address_bindings) = match row.reference.property_identifier {
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS => (true, false, false),
            PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS => (false, true, false),
            PropertyIdentifier::DEVICE_ADDRESS_BINDING => (false, false, true),
            _ => return,
        };
        if *device.get_or_insert_with(|| db.selected_device()) != Some(oid) {
            return;
        }
        let selection = selection.get_or_insert(LiveCovSelection {
            device: oid,
            active: false,
            multiple: false,
            address_bindings: false,
        });
        selection.active |= active;
        selection.multiple |= multiple;
        selection.address_bindings |= address_bindings;
    };
    for (oid, row) in rows {
        select(oid, row);
        for (member, member_row) in row.members.iter().flat_map(GroupMembers::rows) {
            select(member, member_row);
        }
    }
    selection
}

/// A ReadPropertyMultiple request planned before any value is read: every
/// result row after target resolution and expansion, a Group's member rows
/// included, charged to the request's work limit. Planning reads no value, so
/// the request takes its one COV snapshot from the plan, then reads (#1213).
pub(crate) struct RpmPlan(Vec<PlannedObject>);

impl RpmPlan {
    /// Plan `request` within `limit` result rows. A request past it fails
    /// with [`ReadFailure::Work`] here, before any value is read.
    pub(crate) fn new(
        db: &ObjectDatabase,
        request: &ReadPropertyMultipleRequest,
        limit: usize,
        view: Option<&DeviceReadContext<'_>>,
    ) -> Result<Self, ReadFailure> {
        plan(db, request, limit, view).map(Self)
    }

    /// The selected Device's COV lists the planned rows read.
    pub(crate) fn live_cov(&self, db: &ObjectDatabase) -> Option<LiveCovSelection> {
        live_cov_selection(db, planned_rows(&self.0))
    }

    /// Read every planned row into `buf`, within `max_service_ack_bytes`.
    ///
    /// Atomic with respect to the caller's buffer, not object read side
    /// effects. Observations are provisional until this entire call succeeds.
    /// The caller must discard them on failure; callbacks carry the requested
    /// index (which may differ from the response index) and no property
    /// values. `view` owns the effective Device definitions and the
    /// request-local COV values for every row.
    pub(crate) fn read_observed(
        self,
        db: &ObjectDatabase,
        view: Option<&DeviceReadContext<'_>>,
        buf: &mut BytesMut,
        max_service_ack_bytes: usize,
        mut completed: impl FnMut(
            ObjectIdentifier,
            PropertyIdentifier,
            Option<u32>,
            Option<(ErrorClass, ErrorCode)>,
        ),
    ) -> Result<(), ReadFailure> {
        let mut scratch = Scratch {
            bytes: BytesMut::new(),
            limit: max_service_ack_bytes,
        };
        let mut footer = BytesMut::new();
        ReadAccessResult::encode_footer(&mut footer);
        for spec in self.0 {
            let mut header = BytesMut::new();
            ReadAccessResult::encode_header(&mut header, &spec.lookup_oid);
            scratch.append(&header, footer.len())?;
            for row in spec.properties {
                let object = read_property::read_target_object(db, &spec.lookup_oid);
                let served = object.and_then(|object| view.map(|view| view.object(object)));
                let object = served
                    .as_ref()
                    .map(|served| served as &dyn BACnetObject)
                    .or(object);
                let requested_index = row.reference.property_array_index;
                let result = read_row(db, view, object, row);
                let mut encoded = BytesMut::new();
                result.encode(&mut encoded);
                scratch.append(&encoded, footer.len())?;
                completed(
                    spec.lookup_oid,
                    result.property_identifier,
                    requested_index,
                    result.error,
                );
            }
            scratch.append(&footer, 0)?;
        }
        buf.extend_from_slice(&scratch.bytes);
        Ok(())
    }
}

fn plan(
    db: &ObjectDatabase,
    request: &ReadPropertyMultipleRequest,
    limit: usize,
    view: Option<&DeviceReadContext<'_>>,
) -> Result<Vec<PlannedObject>, ReadFailure> {
    plan_specs(
        db,
        &request.list_of_read_access_specs,
        &mut Work::new(limit),
        view,
        true,
    )
}

/// Resolve and expand `specs`, charging every row to `work`. With `groups`, a
/// row that reads a Group's whole Present_Value plans the Group's members as
/// well; member rows are planned without it, so they read each object's own
/// value and a Group listed as a member can't make the read recurse.
pub(super) fn plan_specs(
    db: &ObjectDatabase,
    specs: &[ReadAccessSpecification],
    work: &mut Work,
    view: Option<&DeviceReadContext<'_>>,
    groups: bool,
) -> Result<Vec<PlannedObject>, ReadFailure> {
    let mut plan = Vec::new();
    for spec in specs {
        let lookup_oid = read_property::resolve_read_target(
            db,
            &spec.object_identifier,
            view.and_then(|view| view.registered_port),
            view.and_then(|view| view.device_oid()),
        );
        let stored = read_property::read_target_object(db, &lookup_oid);
        let served = stored.and_then(|object| view.map(|view| view.object(object)));
        let object = served
            .as_ref()
            .map(|served| served as &dyn BACnetObject)
            .or(stored);
        let mut properties = Vec::new();
        for reference in &spec.list_of_property_references {
            let mut push = |id| {
                work.charge()?;
                let row = PropertyReference {
                    property_identifier: id,
                    property_array_index: if id == reference.property_identifier {
                        reference.property_array_index
                    } else {
                        None
                    },
                };
                let members = match object {
                    Some(object) if groups => {
                        group_present_value::plan_members(db, view, object, &row, work)?
                    }
                    _ => None,
                };
                properties.push(PlannedRow {
                    reference: row,
                    members,
                });
                Ok(())
            };
            match object {
                Some(object) => expand(object, reference, push)?,
                None => push(reference.property_identifier)?,
            }
        }
        // Empty object wrappers are retained; their count is bounded by the
        // existing decoded request, not by the expanded-result policy.
        plan.push(PlannedObject {
            lookup_oid,
            properties,
        });
    }
    Ok(plan)
}

/// Read one planned row. `object` is the effective read view of the row's
/// target, if it exists; a Group's Present_Value row is built from the members
/// it planned, every other row is the object's own answer.
pub(super) fn read_row(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    object: Option<&dyn BACnetObject>,
    row: PlannedRow,
) -> ReadResultElement {
    let PlannedRow { reference, members } = row;
    element(object, &reference, |object| match members {
        Some(members) => group_present_value::value(db, view, members),
        None => object.read_property(
            reference.property_identifier,
            reference.property_array_index,
        ),
    })
}

/// `object` is the effective read view for this row, if the object exists.
/// Lookup and canonical array-index precedence match ReadProperty. `read`
/// fetches the row's value from the object once those checks pass.
pub(super) fn element(
    object: Option<&dyn BACnetObject>,
    reference: &PropertyReference,
    read: impl FnOnce(&dyn BACnetObject) -> Result<PropertyValue, Error>,
) -> ReadResultElement {
    let id = reference.property_identifier;
    let index = reference.property_array_index;
    let response_index = super::read_property::rpm_response_index(object, id, index);
    let result = match object {
        None => Err((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
        Some(object) if index.is_some() && !object.is_array_property(id) => {
            Err((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
        }
        Some(object) => match read(object) {
            Ok(value) => {
                // One property may return/encode an arbitrarily large owned
                // value. Only accumulated service bytes are bounded here.
                let mut encoded = BytesMut::new();
                encode_property_value(&mut encoded, &value)
                    .map(|()| encoded.to_vec())
                    .map_err(|_| (ErrorClass::PROPERTY, ErrorCode::OTHER))
            }
            Err(Error::Protocol { class, code }) => Err((class, code)),
            Err(_) => Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        },
    };
    let (property_value, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    ReadResultElement {
        property_identifier: id,
        property_array_index: response_index,
        property_value,
        error,
    }
}

struct Scratch {
    bytes: BytesMut,
    limit: usize,
}

impl Scratch {
    fn append(&mut self, bytes: &[u8], reserved: usize) -> Result<(), ReadFailure> {
        self.bytes
            .len()
            .checked_add(bytes.len())
            .and_then(|n| n.checked_add(reserved))
            .filter(|&n| n <= self.limit)
            .ok_or(ReadFailure::Bytes)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

/// Atomic with respect to the caller's buffer, not object read side effects.
#[cfg(test)]
pub(crate) fn handle_rpm_budgeted(
    db: &ObjectDatabase,
    data: &[u8],
    buf: &mut BytesMut,
    budget: ReadPropertyMultipleBudget,
) -> Result<(), ReadFailure> {
    handle_rpm_budgeted_observed(db, data, buf, budget, |_, _, _, _| {})
}

/// Decode, then evaluate without a server COV context.
#[cfg(test)]
pub(crate) fn handle_rpm_budgeted_observed(
    db: &ObjectDatabase,
    data: &[u8],
    buf: &mut BytesMut,
    budget: ReadPropertyMultipleBudget,
    completed: impl FnMut(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        Option<(ErrorClass, ErrorCode)>,
    ),
) -> Result<(), ReadFailure> {
    let request = ReadPropertyMultipleRequest::decode(data)
        .map_err(|error| ReadFailure::Service(error.into_request_reject()))?;
    RpmPlan::new(db, &request, budget.max_result_elements, None)?.read_observed(
        db,
        None,
        buf,
        budget.max_service_ack_bytes,
        completed,
    )
}

#[cfg(test)]
#[path = "tests/rpm_budget.rs"]
mod tests;
