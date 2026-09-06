// ===========================================================================
// Lighting enums (Clause 12.54, 12.55)
// ===========================================================================

bacnet_enum! {
    /// BACnet binary lighting present value (Clause 21).
    pub struct BinaryLightingPV(u32);

    const OFF = 0;
    const ON = 1;
    const WARN = 2;
    const WARN_OFF = 3;
    const WARN_RELINQUISH = 4;
    const STOP = 5;
}

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

bacnet_enum! {
    /// BACnet lighting transition kind (Clause 21).
    pub struct LightingTransition(u32);

    const NONE = 0;
    const FADE = 1;
    const RAMP = 2;
}
