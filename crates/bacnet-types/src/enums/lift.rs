// ===========================================================================
// Lift / escalator enums (Clause 12.58-12.60)
// ===========================================================================

bacnet_enum! {
    /// BACnet escalator operating mode (Clause 12.60).
    pub struct EscalatorMode(u32);

    const UNKNOWN = 0;
    const STOP = 1;
    const UP = 2;
    const DOWN = 3;
    const INSPECTION = 4;
    const OUT_OF_SERVICE = 5;
}

bacnet_enum! {
    /// BACnet escalator fault signals (Clause 12.60).
    pub struct EscalatorFault(u32);

    const CONTROLLER_FAULT = 0;
    const DRIVE_AND_MOTOR_FAULT = 1;
    const MECHANICAL_COMPONENT_FAULT = 2;
    const OVERSPEED_FAULT = 3;
    const POWER_SUPPLY_FAULT = 4;
    const SAFETY_DEVICE_FAULT = 5;
    const CONTROLLER_SUPPLY_FAULT = 6;
    const DRIVE_TEMPERATURE_EXCEEDED = 7;
    const COMB_PLATE_FAULT = 8;
}

bacnet_enum! {
    /// BACnet lift car travel direction (Clause 12.59).
    pub struct LiftCarDirection(u32);

    const UNKNOWN = 0;
    const NONE = 1;
    const STOPPED = 2;
    const UP = 3;
    const DOWN = 4;
    const UP_AND_DOWN = 5;
}

bacnet_enum! {
    /// BACnet lift group operating mode (Clause 12.58).
    pub struct LiftGroupMode(u32);

    const UNKNOWN = 0;
    const NORMAL = 1;
    const DOWN_PEAK = 2;
    const TWO_WAY = 3;
    const FOUR_WAY = 4;
    const EMERGENCY_POWER = 5;
    const UP_PEAK = 6;
}

bacnet_enum! {
    /// BACnet lift car door status (Clause 12.59).
    pub struct LiftCarDoorStatus(u32);

    const UNKNOWN = 0;
    const NONE = 1;
    const CLOSING = 2;
    const CLOSED = 3;
    const OPENING = 4;
    const OPENED = 5;
    const SAFETY_LOCKED = 6;
    const LIMITED_OPENED = 7;
}

bacnet_enum! {
    /// BACnet lift car door command (Clause 21).
    pub struct LiftCarDoorCommand(u32);

    const NONE = 0;
    const OPEN = 1;
    const CLOSE = 2;
}

bacnet_enum! {
    /// BACnet lift car drive status (Clause 21).
    pub struct LiftCarDriveStatus(u32);

    const UNKNOWN = 0;
    const STATIONARY = 1;
    const BRAKING = 2;
    const ACCELERATE = 3;
    const DECELERATE = 4;
    const RATED_SPEED = 5;
    const SINGLE_FLOOR_JUMP = 6;
    const TWO_FLOOR_JUMP = 7;
    const THREE_FLOOR_JUMP = 8;
    const MULTI_FLOOR_JUMP = 9;
}

bacnet_enum! {
    /// BACnet lift car operating mode (Clause 21).
    pub struct LiftCarMode(u32);

    const UNKNOWN = 0;
    const NORMAL = 1;
    const VIP = 2;
    const HOMING = 3;
    const PARKING = 4;
    const ATTENDANT_CONTROL = 5;
    const FIREFIGHTER_CONTROL = 6;
    const EMERGENCY_POWER = 7;
    const INSPECTION = 8;
    const CABINET_RECALL = 9;
    const EARTHQUAKE_OPERATION = 10;
    const FIRE_OPERATION = 11;
    const OUT_OF_SERVICE = 12;
    const OCCUPANT_EVACUATION = 13;
}

bacnet_enum! {
    /// BACnet lift fault signals (Clause 21).
    pub struct LiftFault(u32);

    const CONTROLLER_FAULT = 0;
    const DRIVE_AND_MOTOR_FAULT = 1;
    const GOVERNOR_AND_SAFETY_GEAR_FAULT = 2;
    const LIFT_SHAFT_DEVICE_FAULT = 3;
    const POWER_SUPPLY_FAULT = 4;
    const SAFETY_INTERLOCK_FAULT = 5;
    const DOOR_CLOSING_FAULT = 6;
    const DOOR_OPENING_FAULT = 7;
    const CAR_STOPPED_OUTSIDE_LANDING_ZONE = 8;
    const CALL_BUTTON_STUCK = 9;
    const START_FAILURE = 10;
    const CONTROLLER_SUPPLY_FAULT = 11;
    const SELF_TEST_FAILURE = 12;
    const RUNTIME_LIMIT_EXCEEDED = 13;
    const POSITION_LOST = 14;
    const DRIVE_TEMPERATURE_EXCEEDED = 15;
    const LOAD_MEASUREMENT_FAULT = 16;
}
