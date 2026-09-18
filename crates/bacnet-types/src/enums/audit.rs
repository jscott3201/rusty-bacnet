// ===========================================================================
// Audit enums (new in 135-2020)
// ===========================================================================

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

impl BACnetSuccessFilter {
    /// Map the pre-RB-02 Boolean query meaning to the corrected filter.
    ///
    /// The uncorrected contract encoded a BOOLEAN `successful-actions-only`:
    /// `true` selected successes-only and `false` selected all. This helper
    /// preserves that source-level meaning while moving callers to the
    /// corrected `BACnetSuccessFilter` type. It accepts exactly one meaning
    /// per call, touches no wire bytes, and performs no encoding or decoding.
    ///
    /// RB-20 completed the migration inventory: 3-state runtime filtering
    /// including `FAILURES_ONLY` behavior plus storage-predicate
    /// re-verification (`crates/bacnet-objects/src/audit.rs`
    /// `operation_matches`/`query_matches`); the Python boundary
    /// (`crates/rusty-bacnet/src/types/audit.rs`, `rusty_bacnet.pyi`, and
    /// `crates/rusty-bacnet/tests/test_audit_api.py`); and user-facing docs
    /// (`rust-api.md`, `python-api.md`, `CHANGELOG`, PICS). Issue #345 stays
    /// open for reporting/forwarding (RB-21/22).
    #[deprecated(
        note = "RB-02 migration aid only: maps the old Boolean meaning (true = successes-only, false = all) to BACnetSuccessFilter; prefer the named ALL / SUCCESSES_ONLY / FAILURES_ONLY constants for new code"
    )]
    pub fn from_legacy_bool(successful_actions_only: bool) -> Self {
        if successful_actions_only {
            Self::SUCCESSES_ONLY
        } else {
            Self::ALL
        }
    }
}
