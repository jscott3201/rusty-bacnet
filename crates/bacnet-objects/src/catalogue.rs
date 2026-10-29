//! The object catalogue: one object of each standard type this crate builds.
//!
//! [`representatives`] constructs one object per [`ObjectType`] that has an
//! implementation here, with fixed small configurations (instance 1, a short
//! name, minimal sizes). It answers "which object types can this crate
//! construct", for audits, generated documentation and sample databases. It
//! says nothing about how well each type behaves on a network: that is what the
//! object and server tests cover.
//!
//! The types without an implementation are listed in [`EXCLUDED_TYPES`]:
//! Network Security (type 38), whose clause was deleted from the standard. A
//! catalogue entry is a starting point to configure, not a recommended
//! deployment: the Audit Log keeps its records in memory only, and every
//! object uses placeholder units and names.
//!
//! The catalogue builds without the standard library, like the crate.

use crate::prelude::*;
use crate::sync::Mutex;
use alloc::sync::Arc;

use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{EngineeringUnits, EventType, NetworkType, ObjectType};
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
use crate::channel::ChannelObject;
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
use crate::notification_forwarder::NotificationForwarderObject;
use crate::program::ProgramObject;
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

/// Standard object types with no implementation in this crate, so no
/// catalogue entry: Network Security (38), whose clause the standard deleted.
pub const EXCLUDED_TYPES: &[ObjectType] = &[ObjectType::NETWORK_SECURITY];

/// Audit Log storage that keeps the last committed snapshot in memory, so the
/// catalogue's Audit Log needs no file system.
#[derive(Default)]
struct MemoryAuditLogPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryAuditLogPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock() = Some(snapshot.clone());
        Ok(())
    }
}

/// A three-stage Staging configuration over two local outputs.
pub(crate) fn staging_config() -> StagingConfig {
    let output = |object_type, instance| BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(object_type, instance)
            .expect("a valid catalogue reference"),
    };
    StagingConfig {
        present_value: 5.0,
        min_present_value: -1.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
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
            output(ObjectType::BINARY_OUTPUT, 1),
            output(ObjectType::BINARY_VALUE, 2),
        ],
        stage_names: Some(vec!["Low".into(), "Middle".into(), "High".into()]),
    }
}

/// One object of every standard type this crate implements, in object type
/// order: every type from Analog Input (0) to Color Temperature (64) except
/// those in [`EXCLUDED_TYPES`].
///
/// Each object has instance 1 and a name unique within the list, so the whole
/// list fits one [`ObjectDatabase`](crate::database::ObjectDatabase). The
/// Device is built from its default configuration. The Staging object starts
/// with Present_Stage uninitialized, as a new one does; a server initializes it
/// at startup.
///
/// The list counts constructible types. It is not a statement about
/// interoperability or about which services reach each type.
///
/// # Panics
///
/// Never in practice: each construction uses fixed arguments that the crate's
/// tests build. A panic here is a bug in this function.
pub fn representatives() -> Vec<Box<dyn BACnetObject>> {
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    let alert_source =
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).expect("a valid catalogue reference");
    let audit_storage: Arc<dyn AuditLogPersistence> =
        Arc::new(MemoryAuditLogPersistence::default());
    let objects: Vec<Result<Box<dyn BACnetObject>, Error>> = vec![
        boxed(AnalogInputObject::new(1, "AI-1", celsius)),
        boxed(AnalogOutputObject::new(1, "AO-1", celsius)),
        boxed(AnalogValueObject::new(1, "AV-1", celsius)),
        boxed(BinaryInputObject::new(1, "BI-1")),
        boxed(BinaryOutputObject::new(1, "BO-1")),
        boxed(BinaryValueObject::new(1, "BV-1")),
        boxed(CalendarObject::new(1, "CAL-1")),
        boxed(CommandObject::new(1, "CMD-1")),
        boxed(DeviceObject::new(Default::default())),
        boxed(EventEnrollmentObject::new(
            1,
            "EE-1",
            EventType::CHANGE_OF_BITSTRING,
        )),
        boxed(FileObject::new(1, "FILE-1", "raw")),
        boxed(GroupObject::new(1, "GRP-1")),
        boxed(LoopObject::new(1, "LOOP-1", celsius)),
        boxed(MultiStateInputObject::new(1, "MSI-1", 3)),
        boxed(MultiStateOutputObject::new(1, "MSO-1", 3)),
        boxed(NotificationClass::new(1, "NC-1")),
        boxed(ProgramObject::new(1, "PRG-1")),
        boxed(ScheduleObject::new(1, "SCH-1", PropertyValue::Null)),
        boxed(AveragingObject::new(1, "AVG-1")),
        boxed(MultiStateValueObject::new(1, "MSV-1", 3)),
        boxed(TrendLogObject::new(1, "TL-1", 3)),
        boxed(LifeSafetyPointObject::new(1, "LSP-1")),
        boxed(LifeSafetyZoneObject::new(1, "LSZ-1")),
        boxed(AccumulatorObject::new(
            1,
            "ACC-1",
            EngineeringUnits::NO_UNITS,
        )),
        boxed(PulseConverterObject::new(1, "PC-1", celsius)),
        boxed(EventLogObject::new(1, "EL-1", 3)),
        boxed(GlobalGroupObject::new(1, "GG-1")),
        boxed(TrendLogMultipleObject::new(1, "TLM-1", 3)),
        boxed(LoadControlObject::new(1, "LC-1")),
        boxed(StructuredViewObject::new(1, "SV-1")),
        boxed(AccessDoorObject::new(1, "DOOR-1")),
        boxed(TimerObject::new(1, "TMR-1")),
        boxed(AccessCredentialObject::new(1, "CRED-1")),
        boxed(AccessPointObject::new(1, "AP-1")),
        boxed(AccessRightsObject::new(1, "AR-1")),
        boxed(AccessUserObject::new(1, "USER-1")),
        boxed(AccessZoneObject::new(1, "ZONE-1")),
        boxed(CredentialDataInputObject::new(1, "CDI-1")),
        // 38, Network Security: in EXCLUDED_TYPES.
        boxed(BitStringValueObject::new(1, "BSV-1")),
        boxed(CharacterStringValueObject::new(1, "CSV-1")),
        boxed(DatePatternValueObject::new(1, "DPV-1")),
        boxed(DateValueObject::new(1, "DV-1")),
        boxed(DateTimePatternValueObject::new(1, "DTPV-1")),
        boxed(DateTimeValueObject::new(1, "DTV-1")),
        boxed(IntegerValueObject::new(1, "IV-1")),
        boxed(LargeAnalogValueObject::new(1, "LAV-1")),
        boxed(OctetStringValueObject::new(1, "OSV-1")),
        boxed(PositiveIntegerValueObject::new(1, "PIV-1")),
        boxed(TimePatternValueObject::new(1, "TPV-1")),
        boxed(TimeValueObject::new(1, "TV-1")),
        boxed(NotificationForwarderObject::new(1, "NF-1")),
        boxed(AlertEnrollmentObject::new(1, "AE-1", alert_source)),
        boxed(ChannelObject::new(1, "CH-1", 1)),
        boxed(LightingOutputObject::new(1, "LO-1")),
        boxed(BinaryLightingOutputObject::new(1, "BLO-1")),
        boxed(NetworkPortObject::new_non_bip(
            1,
            "NP-1",
            NetworkType::from_raw(0),
            0,
            Default::default(),
            1476,
        )),
        boxed(ElevatorGroupObject::new(1, "EG-1")),
        boxed(EscalatorObject::new(1, "ESC-1")),
        boxed(LiftObject::new(1, "LIFT-1", 3)),
        boxed(StagingObject::new(1, "STG-1", staging_config())),
        boxed(AuditLogObject::new(1, "AL-1", 3, audit_storage)),
        boxed(AuditReporterObject::new(1, "ARP-1")),
        boxed(ColorObject::new(1, "CLR-1")),
        boxed(ColorTemperatureObject::new(1, "CT-1")),
    ];
    objects
        .into_iter()
        .map(|object| object.expect("a catalogue object builds from its fixed arguments"))
        .collect()
}

fn boxed<O: BACnetObject + 'static>(
    object: Result<O, Error>,
) -> Result<Box<dyn BACnetObject>, Error> {
    object.map(|object| Box::new(object) as Box<dyn BACnetObject>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_object_per_type_except_the_excluded() {
        let objects = representatives();
        let types: Vec<u32> = objects
            .iter()
            .map(|object| object.object_identifier().object_type().to_raw())
            .collect();
        let excluded: Vec<u32> = EXCLUDED_TYPES.iter().map(|t| t.to_raw()).collect();
        let expected: Vec<u32> = (0..=64).filter(|raw| !excluded.contains(raw)).collect();
        assert_eq!(types, expected, "one object per type, in type order");
    }

    #[test]
    fn the_whole_catalogue_fits_one_database() {
        let objects = representatives();
        let count = objects.len();
        let mut database = crate::database::ObjectDatabase::new();
        for object in objects {
            let oid = object.object_identifier();
            database
                .add(object)
                .unwrap_or_else(|error| panic!("{oid:?} must fit beside the others: {error:?}"));
        }
        assert_eq!(database.len(), count);
    }
}
