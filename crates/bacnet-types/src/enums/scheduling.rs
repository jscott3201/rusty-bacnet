// ===========================================================================
// Timer enums (Clause 12.31)
// ===========================================================================

bacnet_enum! {
    /// BACnet timer state (Clause 12.31, new in 135-2020).
    pub struct TimerState(u32);

    const IDLE = 0;
    const RUNNING = 1;
    const EXPIRED = 2;
}

bacnet_enum! {
    /// BACnet timer state transition (Clause 12.31, new in 135-2020).
    pub struct TimerTransition(u32);

    const NONE = 0;
    const IDLE_TO_RUNNING = 1;
    const RUNNING_TO_IDLE = 2;
    const RUNNING_TO_RUNNING = 3;
    const RUNNING_TO_EXPIRED = 4;
    const FORCED_TO_EXPIRED = 5;
    const EXPIRED_TO_IDLE = 6;
    const EXPIRED_TO_RUNNING = 7;
}
