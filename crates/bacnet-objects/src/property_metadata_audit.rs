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
//! - `CHANNEL` (53) and `NOTIFICATION_FORWARDER` (51) are unsupported
//!   by design: no `impl BACnetObject` exists, the Device support bits stay
//!   clear (`device/tests.rs` pins bits 51/53 clear), and the conformance
//!   ledger records them as unsupported-by-design.
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
//!   The `Optional` + `IntrinsicReporting` modeling below matches that
//!   conditional-optional classification exactly.
//! - Table 12-14 (Event Enrollment, p. 232) and Table 12-61
//!   (Alert Enrollment, p. 504) mark Event_Time_Stamps unconditionally
//!   Required, which the `RequiredRead` rows below match.
//!
//! If the Standard ever reclassified these rows, the flip would change
//! PICS/RPM-REQUIRED output and needs an owner compatibility decision first;
//! do not "fix" a pin failure by editing the pins.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::access_control::{
    AccessCredentialObject, AccessDoorObject, AccessPointObject, AccessRightsObject,
    AccessUserObject, AccessZoneObject, CredentialDataInputObject,
};
use crate::accumulator::{AccumulatorObject, PulseConverterObject};
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot, AuditReporterObject};
use crate::averaging::AveragingObject;
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::color::{ColorObject, ColorTemperatureObject};
use crate::command::CommandObject;
use crate::device::DeviceObject;
use crate::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use crate::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use crate::event_log::EventLogObject;
use crate::file::FileObject;
use crate::group::{GlobalGroupObject, GroupObject, StructuredViewObject};
use crate::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use crate::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use crate::load_control::LoadControlObject;
use crate::loop_obj::LoopObject;
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::network_port::NetworkPortObject;
use crate::notification_class::NotificationClass;
use crate::program::ProgramObject;
use crate::property_metadata::{
    required_properties_from_metadata, PropertyConformance, PropertyPresenceCondition,
};
use crate::schedule::{CalendarObject, ScheduleObject};
use crate::staging::{StagingConfig, StagingObject};
use crate::timer::TimerObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};
use crate::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};

#[derive(Default)]
struct MemoryAuditLogPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryAuditLogPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn staging_config() -> StagingConfig {
    StagingConfig {
        present_value: 5.0,
        min_present_value: -1.0,
        units: 62,
        priority_for_writing: 8,
        stages: vec![
            BACnetStageLimitValue {
                limit: 10.0,
                values: vec![false, true],
                deadband: 1.0,
            },
            BACnetStageLimitValue {
                limit: 20.0,
                values: vec![true, false],
                deadband: 2.0,
            },
            BACnetStageLimitValue {
                limit: 30.0,
                values: vec![true, true],
                deadband: 1.0,
            },
        ],
        target_references: vec![
            BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            },
            BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2).unwrap(),
            },
        ],
        stage_names: Some(vec!["Low".into(), "Middle".into(), "High".into()]),
    }
}

/// One representative per supported `ObjectType`: 62 total.
///
/// Excluded by design (see module docs): 51 NOTIFICATION_FORWARDER, 53
/// CHANNEL, 38 NETWORK_SECURITY.
fn supported_representatives() -> Vec<Box<dyn BACnetObject>> {
    let alert_source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    vec![
        Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()),
        Box::new(AnalogOutputObject::new(1, "AO-1", 62).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV-1", 62).unwrap()),
        Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        Box::new(CalendarObject::new(1, "CAL-1").unwrap()),
        Box::new(CommandObject::new(1, "CMD-1").unwrap()),
        Box::new(DeviceObject::new(Default::default()).unwrap()),
        Box::new(EventEnrollmentObject::new(1, "EE-1", 0).unwrap()),
        Box::new(FileObject::new(1, "FILE-1", "raw").unwrap()),
        Box::new(GroupObject::new(1, "GRP-1").unwrap()),
        Box::new(LoopObject::new(1, "LOOP-1", 62).unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
        Box::new(NotificationClass::new(1, "NC-1").unwrap()),
        Box::new(ProgramObject::new(1, "PRG-1").unwrap()),
        Box::new(ScheduleObject::new(1, "SCH-1", PropertyValue::Null).unwrap()),
        Box::new(AveragingObject::new(1, "AVG-1").unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(TrendLogObject::new(1, "TL-1", 3).unwrap()),
        Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()),
        Box::new(LifeSafetyZoneObject::new(1, "LSZ-1").unwrap()),
        Box::new(AccumulatorObject::new(1, "ACC-1", 95).unwrap()),
        Box::new(PulseConverterObject::new(1, "PC-1", 62).unwrap()),
        Box::new(EventLogObject::new(1, "EL-1", 3).unwrap()),
        Box::new(GlobalGroupObject::new(1, "GG-1").unwrap()),
        Box::new(TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap()),
        Box::new(LoadControlObject::new(1, "LC-1").unwrap()),
        Box::new(StructuredViewObject::new(1, "SV-1").unwrap()),
        Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()),
        Box::new(TimerObject::new(1, "TMR-1").unwrap()),
        Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()),
        Box::new(AccessPointObject::new(1, "AP-1").unwrap()),
        Box::new(AccessRightsObject::new(1, "AR-1").unwrap()),
        Box::new(AccessUserObject::new(1, "USER-1").unwrap()),
        Box::new(AccessZoneObject::new(1, "ZONE-1").unwrap()),
        Box::new(CredentialDataInputObject::new(1, "CDI-1").unwrap()),
        // 38 NETWORK_SECURITY: deprecated, no impl — excluded.
        Box::new(BitStringValueObject::new(1, "BSV-1").unwrap()),
        Box::new(CharacterStringValueObject::new(1, "CSV-1").unwrap()),
        Box::new(DatePatternValueObject::new(1, "DPV-1").unwrap()),
        Box::new(DateValueObject::new(1, "DV-1").unwrap()),
        Box::new(DateTimePatternValueObject::new(1, "DTPV-1").unwrap()),
        Box::new(DateTimeValueObject::new(1, "DTV-1").unwrap()),
        Box::new(IntegerValueObject::new(1, "IV-1").unwrap()),
        Box::new(LargeAnalogValueObject::new(1, "LAV-1").unwrap()),
        Box::new(OctetStringValueObject::new(1, "OSV-1").unwrap()),
        Box::new(PositiveIntegerValueObject::new(1, "PIV-1").unwrap()),
        Box::new(TimePatternValueObject::new(1, "TPV-1").unwrap()),
        Box::new(TimeValueObject::new(1, "TV-1").unwrap()),
        // 51 NOTIFICATION_FORWARDER: unsupported by design — excluded.
        Box::new(AlertEnrollmentObject::new(1, "AE-1", alert_source).unwrap()),
        // 53 CHANNEL: unsupported by design — excluded.
        Box::new(LightingOutputObject::new(1, "LO-1").unwrap()),
        Box::new(BinaryLightingOutputObject::new(1, "BLO-1").unwrap()),
        Box::new(NetworkPortObject::new(1, "NP-1", 0).unwrap()),
        Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()),
        Box::new(EscalatorObject::new(1, "ESC-1").unwrap()),
        Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
        Box::new(StagingObject::new(1, "STG-1", staging_config()).unwrap()),
        Box::new(
            AuditLogObject::new(1, "AL-1", 3, Arc::new(MemoryAuditLogPersistence::default()))
                .unwrap(),
        ),
        Box::new(AuditReporterObject::new(1, "AR-1").unwrap()),
        Box::new(ColorObject::new(1, "CLR-1").unwrap()),
        Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
    ]
}

fn audit_object_type_coverage(objects: &[Box<dyn BACnetObject>]) {
    let mut raws: Vec<u32> = objects
        .iter()
        .map(|object| object.object_identifier().object_type().to_raw())
        .collect();
    raws.sort_unstable();
    let expected: Vec<u32> = (0..=64).filter(|raw| ![38, 51, 53].contains(raw)).collect();
    assert_eq!(
        raws, expected,
        "audit must cover exactly the 62 supported types"
    );
}

#[test]
fn audit_every_supported_type_derives_from_canonical_metadata() {
    let mut objects = supported_representatives();
    assert_eq!(objects.len(), 62, "one representative per supported type");
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
                object.read_property(row.property_identifier, None).is_ok(),
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
    // intrinsic-reporting condition (see module docs for pages).
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
            Some(PropertyPresenceCondition::IntrinsicReporting),
            "{object_type:?} EVENT_TIME_STAMPS must stay intrinsic-reporting conditional"
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
