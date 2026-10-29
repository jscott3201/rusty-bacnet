use super::{ElevatorGroupObject, EscalatorObject, LiftObject};
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Elevator trio (ASHRAE 135-2020; PDF = printed + 2):
// - ElevatorGroup (type 57, §12.58 Table 12-76; printed p. 578 / PDF p. 580)
// - Lift (type 59, §12.59 Table 12-77; printed pp. 583-584 / PDF pp. 585-586)
// - Escalator (type 58, §12.60 Table 12-78; printed p. 594 / PDF p. 596)
// ElevatorGroup keeps its legacy order with Machine_Room_ID after
// Object_Type, its Table 12-76 neighbour among the served rows (#997). Lift
// and Escalator list their rows in table order (#1021, #1022). PROPERTY_LIST
// is last so the projection helper omits it while required_properties keeps
// it. Only implemented rows are described: table rows the objects do not
// serve (ElevatorGroup audit/tag/profile rows; Escalator event, intrinsic,
// audit, tag and profile rows; Lift Car_Door_Text, deck, event, intrinsic,
// audit, tag and profile rows) stay absent until dispatch exists. The Lift
// serves Energy_Meter_Ref after Energy_Meter, its Table 12-77 neighbour
// (#1036).
// Every Lift and Escalator row the objects serve is a table row: the Lift's
// former Tracking_Value and Floor_Number are gone (#1021).
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED. Description carries the table O code with a routed
// CharacterString write arm, so Optional/Always. Table-R served rows with no
// network write route stay RequiredRead/ReadOnly; table-R rows with a write
// arm are RequiredRead/Always. Table-O served rows are Optional, with Always
// exactly where dispatch accepts the write.
// Elevator_Group, Group_ID and Installation_ID are R rows of both Tables
// 12-77 and 12-78. They are application-owned and read-only over the
// network (membership.rs), unlike the Elevator Group's own writable Group_ID.
// The Lift's Car_Load_Units (O, present exactly when Car_Load is) and both
// objects' Energy_Meter_Ref (O, energy_meter.rs) are likewise set through
// Rust setters only.
// Table 12-76 has no Status_Flags, Out_Of_Service, or Reliability row, so
// ElevatorGroup serves none of them (#997, as #984 did for Calendar). Lift and
// Escalator Tables 12-77/12-78 do list them (Status_Flags R, Out_Of_Service
// R, Reliability O): Status_Flags and Reliability are RequiredRead/ReadOnly
// and Optional/ReadOnly, and Out_Of_Service is RequiredRead/Always through
// its routed Boolean arm.
// Item (c) of the Lift §12.59 and Escalator §12.60 Out_Of_Service
// descriptions makes their status rows writable while Out_Of_Service is TRUE.
// The status rows that dispatch already routes in service (Car_Position,
// Car_Moving_Direction, Car_Load, Passenger_Alarm, Energy_Meter,
// Fault_Signals, and the Escalator family of #401) stay Always, since the
// writability suites pin in-service writes. The Lift's per-door arrays
// (doors.rs) and its car-state rows Car_Assigned_Direction, Car_Door_Zone,
// Car_Mode, Next_Stopping_Floor and Car_Drive_Status (car_state.rs) are
// application-owned in service and take simulation writes only while out
// of service, as items (c) and (d) ask, so they are WhenOutOfService
// (#1035, #1052). All of those are Table 12-77 O rows.
// Presence is None throughout: the implementation models no
// lift-group-conditional, intrinsic-reporting, or paired-text gating on this
// family, and Car_Load and Car_Load_Units are always served together. The
// trio is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. COV keeps its default. Group_Members admits an
// index through the array default (BACnetARRAY per Table 12-76) and serves
// it through common::read_array (#1034); the Lift
// overrides is_array_property so Floor_Text and the per-door arrays
// (BACnetARRAYs of Table 12-77) admit one too. Every other served row
// rejects an index.
const ELEVATOR_GROUP_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MACHINE_ROOM_ID, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_ID, RequiredRead, None, Always),
    PropertyMetadata::new(P::GROUP_MEMBERS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_MODE, Optional, None, Always),
    PropertyMetadata::new(P::LANDING_CALLS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LANDING_CALL_CONTROL, Optional, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ESCALATOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ELEVATOR_GROUP, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_ID, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::INSTALLATION_ID, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::POWER_MODE, Optional, None, Always),
    PropertyMetadata::new(P::OPERATION_DIRECTION, RequiredRead, None, Always),
    PropertyMetadata::new(P::ESCALATOR_MODE, Optional, None, Always),
    PropertyMetadata::new(P::ENERGY_METER, Optional, None, Always),
    PropertyMetadata::new(P::ENERGY_METER_REF, Optional, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::FAULT_SIGNALS, Optional, None, Always),
    PropertyMetadata::new(P::PASSENGER_ALARM, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const LIFT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ELEVATOR_GROUP, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GROUP_ID, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::INSTALLATION_ID, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::FLOOR_TEXT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::ASSIGNED_LANDING_CALLS, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::MAKING_CAR_CALL, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::REGISTERED_CAR_CALL, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_POSITION, RequiredRead, None, Always),
    PropertyMetadata::new(P::CAR_MOVING_DIRECTION, RequiredRead, None, Always),
    PropertyMetadata::new(P::CAR_ASSIGNED_DIRECTION, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_DOOR_STATUS, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_DOOR_COMMAND, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_DOOR_ZONE, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_MODE, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::CAR_LOAD, Optional, None, Always),
    PropertyMetadata::new(P::CAR_LOAD_UNITS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::NEXT_STOPPING_FLOOR, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::PASSENGER_ALARM, RequiredRead, None, Always),
    PropertyMetadata::new(P::ENERGY_METER, Optional, None, Always),
    PropertyMetadata::new(P::ENERGY_METER_REF, Optional, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::CAR_DRIVE_STATUS, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::FAULT_SIGNALS, RequiredRead, None, Always),
    PropertyMetadata::new(P::LANDING_DOOR_STATUS, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_elevator_group_object(
    _object: &ElevatorGroupObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ELEVATOR_GROUP_BASE)
}

pub(super) fn for_escalator_object(_object: &EscalatorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ESCALATOR_BASE)
}

pub(super) fn for_lift_object(_object: &LiftObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(LIFT_BASE)
}
