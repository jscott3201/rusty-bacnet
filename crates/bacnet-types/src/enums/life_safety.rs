// ===========================================================================
// Life safety enums (Clause 12.15, 12.16)
// ===========================================================================

bacnet_enum! {
    /// Life safety point/zone sensor state (Clause 12.15/12.16).
    pub struct LifeSafetyState(u32);

    const QUIET = 0;
    const PRE_ALARM = 1;
    const ALARM = 2;
    const FAULT = 3;
    const FAULT_PRE_ALARM = 4;
    const FAULT_ALARM = 5;
    const NOT_READY = 6;
    const ACTIVE = 7;
    const TAMPER = 8;
    const TEST_ALARM = 9;
    const TEST_ACTIVE = 10;
    const TEST_FAULT = 11;
    const TEST_FAULT_ALARM = 12;
    const HOLDUP = 13;
    const DURESS = 14;
    const TAMPER_ALARM = 15;
    const ABNORMAL = 16;
    const EMERGENCY_POWER = 17;
    const DELAYED = 18;
    const BLOCKED = 19;
    const LOCAL_ALARM = 20;
    const GENERAL_ALARM = 21;
    const SUPERVISORY = 22;
    const TEST_SUPERVISORY = 23;
}

bacnet_enum! {
    /// Life safety operating mode (Clause 12.15.12).
    pub struct LifeSafetyMode(u32);

    const OFF = 0;
    const ON = 1;
    const TEST = 2;
    const MANNED = 3;
    const UNMANNED = 4;
    const ARMED = 5;
    const DISARMED = 6;
    const PRE_ARMED = 7;
    const SLOW = 8;
    const FAST = 9;
    const DISCONNECTED = 10;
    const ENABLED = 11;
    const DISABLED = 12;
    const AUTOMATIC_RELEASE_DISABLED = 13;
    const DEFAULT = 14;
    const ACTIVATED_OEO_ALARM = 15;
    const ACTIVATED_OEO_EVACUATE = 16;
    const ACTIVATED_OEO_PHASE1_RECALL = 17;
    const ACTIVATED_OEO_UNAVAILABLE = 18;
    const DEACTIVATED = 19;
}

bacnet_enum! {
    /// Life safety commanded operation (Clause 12.15.13, Table 12-54).
    pub struct LifeSafetyOperation(u32);

    const NONE = 0;
    const SILENCE = 1;
    const SILENCE_AUDIBLE = 2;
    const SILENCE_VISUAL = 3;
    const RESET = 4;
    const RESET_ALARM = 5;
    const RESET_FAULT = 6;
    const UNSILENCE = 7;
    const UNSILENCE_AUDIBLE = 8;
    const UNSILENCE_VISUAL = 9;
}

bacnet_enum! {
    /// Silenced state for a life safety point/zone (Clause 12.15.14).
    pub struct SilencedState(u32);

    const UNSILENCED = 0;
    const AUDIBLE_SILENCED = 1;
    const VISIBLE_SILENCED = 2;
    const ALL_SILENCED = 3;
}
