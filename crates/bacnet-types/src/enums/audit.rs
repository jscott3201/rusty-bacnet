// ===========================================================================
// Staging / Audit enums (new in 135-2020)
// ===========================================================================

bacnet_enum! {
    /// BACnet staging state (Clause 12.62, new in 135-2020).
    pub struct StagingState(u32);

    const NOT_STAGED = 0;
    const STAGING = 1;
    const STAGED = 2;
    const COMMITTING = 3;
    const COMMITTED = 4;
    const ABANDONING = 5;
    const ABANDONED = 6;
}

bacnet_enum! {
    /// BACnet audit level (Clause 19.6, new in 135-2020).
    pub struct AuditLevel(u32);

    const NONE = 0;
    const AUDIT_ALL = 1;
    const AUDIT_CONFIG = 2;
    const DEFAULT = 3;
}

bacnet_enum! {
    /// BACnet audit operation (Clause 19.6, new in 135-2020).
    pub struct AuditOperation(u32);

    const READ = 0;
    const WRITE = 1;
    const CREATE = 2;
    const DELETE = 3;
    const LIFE_SAFETY = 4;
    const ACKNOWLEDGE_ALARM = 5;
    const DEVICE_DISABLE_COMM = 6;
    const DEVICE_ENABLE_COMM = 7;
    const DEVICE_RESET = 8;
    const DEVICE_BACKUP = 9;
    const DEVICE_RESTORE = 10;
    const SUBSCRIPTION = 11;
    const NOTIFICATION = 12;
    const AUDITING_FAILURE = 13;
    const NETWORK_CHANGES = 14;
    const GENERAL = 15;
}

bacnet_enum! {
    /// BACnet success filter for audit log queries (Clause 13.19, new in 135-2020).
    pub struct BACnetSuccessFilter(u32);

    const ALL = 0;
    const SUCCESSES_ONLY = 1;
    const FAILURES_ONLY = 2;
}
