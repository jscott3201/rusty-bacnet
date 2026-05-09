// ===========================================================================
// Lighting enums (Clause 12.54)
// ===========================================================================

bacnet_enum! {
    /// BACnet lighting operation (Clause 12.54).
    pub struct LightingOperation(u32);

    const NONE = 0;
    const FADE_TO = 1;
    const RAMP_TO = 2;
    const STEP_UP = 3;
    const STEP_DOWN = 4;
    const STEP_ON = 5;
    const STEP_OFF = 6;
    const WARN = 7;
    const WARN_OFF = 8;
    const WARN_RELINQUISH = 9;
    const STOP = 10;
}

bacnet_enum! {
    /// BACnet lighting in-progress state (Clause 12.54).
    pub struct LightingInProgress(u32);

    const IDLE = 0;
    const FADE_ACTIVE = 1;
    const RAMP_ACTIVE = 2;
    const NOT_CONTROLLED = 3;
    const OTHER = 4;
}
