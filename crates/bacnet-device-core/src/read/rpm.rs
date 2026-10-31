//! ReadPropertyMultiple planning and service-ACK accumulation, within the
//! request's work and byte limits.
use super::budget::{ReadFailure, Work};
use super::group::{self, GroupMembers};
use super::property;
use super::{LiveLists, ReadOverlay, ReadScope};
use alloc::vec::Vec;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleRequest, ReadResultElement};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// One specification after target resolution and ALL/REQUIRED/OPTIONAL
/// expansion; a Group's Present_Value plans its members the same way.
pub(crate) struct PlannedObject {
    pub(crate) lookup_oid: ObjectIdentifier,
    pub(crate) properties: Vec<PlannedRow>,
}

/// One expanded result row. A row that reads a Group's whole Present_Value
/// also carries the Group's members, planned and charged with the row.
pub(crate) struct PlannedRow {
    pub(crate) reference: PropertyReference,
    pub(crate) members: Option<GroupMembers>,
}

/// Visit the properties `reference` expands to on `object`, in order: ALL,
/// REQUIRED and OPTIONAL expand, any other property is itself. `visit`'s
/// error stops the expansion.
// Visit expansion rows without collecting a second, unbounded expansion vector.
// Opaque metadata allocation is outside this service budget.
pub(crate) fn expand(
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
    for row in object.property_metadata().iter() {
        let selected = match id {
            PropertyIdentifier::ALL => row.property_identifier != PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::REQUIRED => {
                row.property_identifier != PropertyIdentifier::PROPERTY_LIST && row.is_required()
            }
            _ => !row.is_required(),
        };
        if selected {
            visit(row.property_identifier)?;
        }
    }
    Ok(())
}

/// Every planned row of `objects`, with the object it reads.
pub(crate) fn planned_rows(
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
pub(crate) fn live_cov_selection<'a>(
    db: &ObjectDatabase,
    rows: impl IntoIterator<Item = (ObjectIdentifier, &'a PlannedRow)>,
) -> Option<LiveLists> {
    // Only a plan that names one of the lists pays the Device lookup.
    let mut device = None;
    let mut selection: Option<LiveLists> = None;
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
        let selection = selection.get_or_insert(LiveLists {
            device: oid,
            active_cov: false,
            active_cov_multiple: false,
            address_bindings: false,
        });
        selection.active_cov |= active;
        selection.active_cov_multiple |= multiple;
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
pub struct RpmPlan(Vec<PlannedObject>);

impl RpmPlan {
    /// Plan `request` within `limit` result rows. A request past it fails
    /// with [`ReadFailure::Work`] here, before any value is read.
    pub fn new<O: ReadOverlay>(
        db: &ObjectDatabase,
        scope: &ReadScope,
        overlay: Option<&O>,
        request: &ReadPropertyMultipleRequest,
        limit: usize,
    ) -> Result<Self, ReadFailure> {
        plan(db, scope, overlay, request, limit).map(Self)
    }

    /// The result rows planned, ALL, REQUIRED and OPTIONAL expanded; a
    /// Group's member rows aren't counted.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> usize {
        planned_rows(&self.0).count()
    }

    /// The selected Device's COV lists the planned rows read.
    pub fn live_lists(&self, db: &ObjectDatabase) -> Option<LiveLists> {
        live_cov_selection(db, planned_rows(&self.0))
    }

    /// Read every planned row into `buf`, within `max_service_ack_bytes`.
    ///
    /// Atomic with respect to the caller's buffer, not object read side
    /// effects. Observations are provisional until this entire call succeeds.
    /// The caller must discard them on failure; callbacks carry the requested
    /// index (which may differ from the response index) and no property
    /// values. `overlay` owns the effective Device definitions and the
    /// request-local COV values for every row.
    pub fn read<O: ReadOverlay>(
        self,
        db: &ObjectDatabase,
        overlay: Option<&O>,
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
                let object = property::read_target_object(db, &spec.lookup_oid);
                let served =
                    object.and_then(|object| overlay.and_then(|overlay| overlay.view(object)));
                let object = served
                    .as_ref()
                    .map(|served| served as &dyn BACnetObject)
                    .or(object);
                let requested_index = row.reference.property_array_index;
                let result = read_row(db, overlay, object, row);
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

fn plan<O: ReadOverlay>(
    db: &ObjectDatabase,
    scope: &ReadScope,
    overlay: Option<&O>,
    request: &ReadPropertyMultipleRequest,
    limit: usize,
) -> Result<Vec<PlannedObject>, ReadFailure> {
    plan_specs(
        db,
        scope,
        overlay,
        &request.list_of_read_access_specs,
        &mut Work::new(limit),
        true,
    )
}

/// Resolve and expand `specs`, charging every row to `work`. With `groups`, a
/// row that reads a Group's whole Present_Value plans the Group's members as
/// well; member rows are planned without it, so they read each object's own
/// value and a Group listed as a member can't make the read recurse.
pub(crate) fn plan_specs<O: ReadOverlay>(
    db: &ObjectDatabase,
    scope: &ReadScope,
    overlay: Option<&O>,
    specs: &[ReadAccessSpecification],
    work: &mut Work,
    groups: bool,
) -> Result<Vec<PlannedObject>, ReadFailure> {
    let mut plan = Vec::new();
    for spec in specs {
        let lookup_oid = property::resolve_target(db, scope, &spec.object_identifier);
        let stored = property::read_target_object(db, &lookup_oid);
        let served = stored.and_then(|object| overlay.and_then(|overlay| overlay.view(object)));
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
                        group::plan_members(db, scope, overlay, object, &row, work)?
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
pub(crate) fn read_row<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    object: Option<&dyn BACnetObject>,
    row: PlannedRow,
) -> ReadResultElement {
    let PlannedRow { reference, members } = row;
    element(object, &reference, |object| match members {
        Some(members) => group::value(db, overlay, members),
        None => object.read_property(
            reference.property_identifier,
            reference.property_array_index,
        ),
    })
}

/// `object` is the effective read view for this row, if the object exists.
/// Lookup and canonical array-index precedence match ReadProperty. `read`
/// fetches the row's value from the object once those checks pass.
pub(crate) fn element(
    object: Option<&dyn BACnetObject>,
    reference: &PropertyReference,
    read: impl FnOnce(&dyn BACnetObject) -> Result<PropertyValue, Error>,
) -> ReadResultElement {
    let id = reference.property_identifier;
    let index = reference.property_array_index;
    let response_index = property::rpm_response_index(object, id, index);
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
            Err(Error::Remote(RemoteError::Error { class, code })) => Err((class, code)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::NoOverlay;
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_types::constructed::ReadAccessSpecification;
    use bacnet_types::enums::EngineeringUnits;

    fn reference(property_identifier: PropertyIdentifier) -> PropertyReference {
        PropertyReference {
            property_identifier,
            property_array_index: None,
        }
    }

    /// ALL, REQUIRED and OPTIONAL visit the metadata rows in order, and the
    /// visitor's error stops the expansion where it is.
    #[test]
    fn expand_follows_the_metadata_and_stops_at_the_visitors_error() {
        let object = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
        let rows = object.property_metadata();
        let optional: Vec<_> = rows
            .iter()
            .filter(|row| !row.is_required())
            .map(|row| row.property_identifier)
            .collect();
        assert!(optional.len() > 3, "{optional:?}");
        let mut visits = Vec::new();
        let result = expand(&object, &reference(PropertyIdentifier::OPTIONAL), |id| {
            visits.push(id);
            if visits.len() == 3 {
                Err(ReadFailure::Work)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(ReadFailure::Work)));
        assert_eq!(visits, optional[..3]);

        let mut all = Vec::new();
        expand(&object, &reference(PropertyIdentifier::ALL), |id| {
            all.push(id);
            Ok(())
        })
        .unwrap();
        assert!(!all.contains(&PropertyIdentifier::PROPERTY_LIST));
        assert_eq!(all.len(), rows.len() - 1);
        let mut one = Vec::new();
        expand(
            &object,
            &reference(PropertyIdentifier::PRESENT_VALUE),
            |id| {
                one.push(id);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(one, [PropertyIdentifier::PRESENT_VALUE]);
    }

    /// A plan counts the expanded rows, and fits a limit of exactly that.
    #[test]
    fn a_plan_counts_its_expanded_rows_against_the_limit() {
        let object = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
        let expanded = object.property_metadata().len() - 1;
        let mut db = ObjectDatabase::new();
        let oid = object.object_identifier();
        db.add(Box::new(object)).unwrap();
        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![
                    reference(PropertyIdentifier::ALL),
                    reference(PropertyIdentifier::PRESENT_VALUE),
                ],
            }],
        };
        let scope = ReadScope::UNLIMITED;
        let plan = RpmPlan::new(&db, &scope, None::<&NoOverlay>, &request, expanded + 1).unwrap();
        assert_eq!(plan.rows(), expanded + 1);
        assert!(matches!(
            RpmPlan::new(&db, &scope, None::<&NoOverlay>, &request, expanded),
            Err(ReadFailure::Work)
        ));
    }

    #[test]
    fn rpm_scratch_growth_never_exceeds_cap_even_on_overflow() {
        let mut scratch = Scratch {
            bytes: BytesMut::new(),
            limit: 4,
        };
        scratch.append(&[1, 2], 2).unwrap();
        assert_eq!(scratch.bytes.len(), 2);
        assert!(matches!(
            scratch.append(&[3, 4, 5], 0),
            Err(ReadFailure::Bytes)
        ));
        assert_eq!(scratch.bytes.len(), 2);
        assert!(matches!(
            scratch.append(&[], usize::MAX),
            Err(ReadFailure::Bytes)
        ));
        scratch.append(&[3, 4], 0).unwrap();
        assert_eq!(scratch.bytes.len(), 4);
    }
}
