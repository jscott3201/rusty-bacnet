//! Workspace-wide property-metadata exactness audit: issue #261 close-out.
//!
//! This test proves that no supported object type uses the trait fallback
//! paths in `traits.rs` (empty `property_metadata` default, the universal-4
//! `required_properties` default, the heuristic `is_writable_property`
//! default). Every supported `ObjectType` gets one representative, and each
//! representative must expose non-empty canonical metadata from which the
//! GENERATED comparison surfaces (`required_properties`, `property_list`,
//! `is_writable_property`) are exactly derived. Any fallback regression or
//! required/optional drift fails this test.
//!
//! Deliberately out of scope (kept, not removed):
//! - The fallback defaults themselves stay for downstream custom objects.
//! - `NETWORK_SECURITY` (38) is deprecated (Clause 24 deleted) with no impl.
//! - `WRITE_GROUP` is a service family, not an object type, so it is not an
//!   `ObjectType` member at all.
//!
//! Event_Time_Stamps pinning (grounded in the local licensed
//! ASHRAE 135-2020 PDF; paraphrased, no verbatim text):
//! - The nine intrinsic-reporting input/output/value tables mark
//!   Event_Time_Stamps with an Optional base code plus the paired
//!   conditional footnotes (required when intrinsic reporting is supported;
//!   present only when it is supported): Tables 12-2 (Analog Input,
//!   printed p. 166), 12-3 (Analog Output, p. 172), 12-4 (Analog Value,
//!   p. 179), 12-6 (Binary Input, p. 190), 12-8 (Binary Output, p. 197),
//!   12-10 (Binary Value, p. 205), 12-21 (Multi-state Input, p. 268),
//!   12-22 (Multi-state Output, p. 275), 12-23 (Multi-state Value, p. 281).
//!   The `Optional` base code with `IntrinsicReportingRequired` below keeps
//!   the table code and records the footnote that makes the row required of
//!   these objects, which all report intrinsically, so RPM REQUIRED and the
//!   PICS list it (#1485).
//! - Table 12-14 (Event Enrollment, p. 232) and Table 12-61
//!   (Alert Enrollment, p. 504) mark Event_Time_Stamps unconditionally
//!   Required, which the `RequiredRead` rows below match.
//!
//! If the Standard ever reclassified these rows, the flip would change
//! PICS/RPM-REQUIRED output and needs an owner compatibility decision first;
//! do not "fix" a pin failure by editing the pins.

use std::collections::HashSet;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

// The submodules build their own objects from these.
use crate::catalogue::{representatives as supported_representatives, staging_config};
use crate::file::FileObject;
use crate::network_port::NetworkPortObject;
use crate::property_metadata::{
    PropertyConformance, PropertyPresenceCondition, required_properties_from_metadata,
};
use crate::staging::StagingObject;
use crate::traits::BACnetObject;

fn audit_object_type_coverage(objects: &[Box<dyn BACnetObject>]) {
    let mut raws: Vec<u32> = objects
        .iter()
        .map(|object| object.object_identifier().object_type().to_raw())
        .collect();
    raws.sort_unstable();
    let expected: Vec<u32> = (0..=64).filter(|raw| *raw != 38).collect();
    assert_eq!(
        raws, expected,
        "audit must cover exactly the 64 supported types"
    );
}

#[test]
fn audit_every_supported_type_derives_from_canonical_metadata() {
    let mut objects = supported_representatives();
    assert_eq!(objects.len(), 64, "one representative per supported type");
    audit_object_type_coverage(&objects);

    // Staging starts with Present_Stage uninitialized (reads
    // VALUE_NOT_INITIALIZED); the bundled server initializes it at startup.
    // Reach that steady state through the public Present_Value write route
    // before auditing row readability.
    for object in objects
        .iter_mut()
        .filter(|object| object.object_identifier().object_type() == ObjectType::STAGING)
    {
        object
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(5.0),
                None,
            )
            .expect("staging representative must accept its startup initialization write");
    }

    for object in &objects {
        let object_type = object.object_identifier().object_type();
        let metadata = object.property_metadata();

        // No supported type may use the empty-metadata fallback.
        assert!(
            !metadata.is_empty(),
            "{object_type:?} must return non-empty canonical metadata"
        );
        assert!(
            metadata
                .iter()
                .any(|row| row.property_identifier == PropertyIdentifier::PROPERTY_LIST),
            "{object_type:?} metadata must contain PROPERTY_LIST"
        );
        let unique: HashSet<_> = metadata.iter().map(|row| row.property_identifier).collect();
        assert_eq!(
            unique.len(),
            metadata.len(),
            "{object_type:?} metadata identifiers must be unique"
        );

        // GENERATED comparison surface: required set derives from R/W rows.
        assert_eq!(
            object.required_properties().as_ref(),
            required_properties_from_metadata(metadata.as_ref()).as_ref(),
            "{object_type:?} required_properties must equal the metadata derivation"
        );

        // GENERATED comparison surface: legacy list is metadata minus PL.
        //
        // One documented exception: Device preserves its legacy projection,
        // which includes PROPERTY_LIST itself (`device/mod.rs`
        // `property_list`). The deviation is contained: PICS (`pics/mod.rs`)
        // and both RPM paths (`handlers/read_property.rs`,
        // `handlers/rpm_budget.rs`) expand migrated objects from metadata
        // directly, so the legacy list only serves direct `property_list()`
        // callers. Pin the deviation exactly here; canonicalizing it is a
        // production behavior change for a separate slice, not this audit.
        if object_type == ObjectType::DEVICE {
            let expected_with_pl: Vec<_> =
                metadata.iter().map(|row| row.property_identifier).collect();
            assert_eq!(
                object.property_list().as_ref(),
                expected_with_pl.as_slice(),
                "{object_type:?} property_list must keep its pinned legacy projection"
            );
        } else {
            let expected_list: Vec<_> = metadata
                .iter()
                .filter_map(|row| {
                    (row.property_identifier != PropertyIdentifier::PROPERTY_LIST)
                        .then_some(row.property_identifier)
                })
                .collect();
            assert_eq!(
                object.property_list().as_ref(),
                expected_list.as_slice(),
                "{object_type:?} property_list must equal metadata minus PROPERTY_LIST"
            );
        }

        // Every advertised row reads, and writability never drifts from dispatch.
        for row in metadata.iter() {
            assert!(
                crate::property_metadata_tests::metadata_row_reads(
                    object.as_ref(),
                    row.property_identifier
                ),
                "{object_type:?} must read {:?} without an array index",
                row.property_identifier
            );
            assert_eq!(
                object.is_writable_property(row.property_identifier),
                row.write_capability.is_writable(),
                "{object_type:?} is_writable must match {:?} write capability",
                row.property_identifier
            );
        }
    }
}

fn event_time_stamps_row(object: &dyn BACnetObject) -> crate::property_metadata::PropertyMetadata {
    *object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == PropertyIdentifier::EVENT_TIME_STAMPS)
        .expect("intrinsic-reporting family must expose EVENT_TIME_STAMPS")
}

#[test]
fn audit_event_time_stamps_classification_matches_clause_12() {
    let objects = supported_representatives();
    audit_object_type_coverage(&objects);
    let by_type = |object_type: ObjectType| -> &dyn BACnetObject {
        objects
            .iter()
            .find(|object| object.object_identifier().object_type() == object_type)
            .map(AsRef::as_ref)
            .expect("supported type must have a representative")
    };

    // Tables 12-2/3/4, 12-6/8/10, 12-21/22/23: Optional base code with the
    // condition intrinsic reporting requires (see module docs for pages).
    for object_type in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        let row = event_time_stamps_row(by_type(object_type));
        assert_eq!(
            row.conformance,
            PropertyConformance::Optional,
            "{object_type:?} EVENT_TIME_STAMPS must stay Optional"
        );
        assert_eq!(
            row.presence_condition,
            Some(PropertyPresenceCondition::IntrinsicReportingRequired),
            "{object_type:?} EVENT_TIME_STAMPS must stay intrinsic-reporting conditional"
        );
        assert!(
            row.is_required(),
            "{object_type:?} EVENT_TIME_STAMPS is required of an intrinsic reporter"
        );
    }

    // Tables 12-14 and 12-61: unconditionally Required.
    for object_type in [ObjectType::EVENT_ENROLLMENT, ObjectType::ALERT_ENROLLMENT] {
        let row = event_time_stamps_row(by_type(object_type));
        assert_eq!(
            row.conformance,
            PropertyConformance::RequiredRead,
            "{object_type:?} EVENT_TIME_STAMPS must stay Required"
        );
        assert_eq!(
            row.presence_condition, None,
            "{object_type:?} EVENT_TIME_STAMPS must stay unconditional"
        );
    }
}

mod unknown_writes;

mod collection_classification;
