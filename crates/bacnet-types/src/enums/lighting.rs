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
    /// Tracking_Value is held inside a Lighting Output's trims while
    /// Present_Value lies outside them, or is following a trim change
    /// (Addendum 135-2020ca part 5).
    const TRIM_ACTIVE = 5;
}

bacnet_enum! {
    /// BACnet lighting transition kind (Clause 21).
    pub struct LightingTransition(u32);

    const NONE = 0;
    const FADE = 1;
    const RAMP = 2;
}

// ===========================================================================
// Color enums (Addendum 135-2020ca)
// ===========================================================================

bacnet_enum! {
    /// BACnet colour operation: the `operation` of a `BACnetColorCommand`
    /// (Addendum 135-2020ca, Clause 21).
    ///
    /// A Color object takes FADE_TO_COLOR and STOP; a Color Temperature
    /// object takes the four CCT operations and STOP. The addendum doesn't
    /// open the enumeration to vendors, so neither object takes any other
    /// value.
    pub struct ColorOperation(u32);

    const NONE = 0;
    const FADE_TO_COLOR = 1;
    const FADE_TO_CCT = 2;
    const RAMP_TO_CCT = 3;
    const STEP_UP_CCT = 4;
    const STEP_DOWN_CCT = 5;
    const STOP = 6;
}

bacnet_enum! {
    /// BACnet colour in-progress state, which both colour object types
    /// report as In_Progress (Addendum 135-2020ca, Clause 21). A Color
    /// object has no ramp, so it never reports RAMP_ACTIVE.
    pub struct ColorOperationInProgress(u32);

    const IDLE = 0;
    const FADE_ACTIVE = 1;
    const RAMP_ACTIVE = 2;
    const NOT_CONTROLLED = 3;
    const OTHER = 4;
}

bacnet_enum! {
    /// BACnet colour transition: how either colour object type moves to a
    /// Present_Value written to it (Addendum 135-2020ca, Clause 21). A Color
    /// object takes NONE and FADE only.
    pub struct ColorTransition(u32);

    const NONE = 0;
    const FADE = 1;
    const RAMP = 2;
}
