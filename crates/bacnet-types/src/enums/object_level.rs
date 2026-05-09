// ===========================================================================
// Object-level enums (Clause 12, 21)
// ===========================================================================

bacnet_enum! {
    /// BACnet event state (Clause 12).
    pub struct EventState(u32);

    const NORMAL = 0;
    const FAULT = 1;
    const OFFNORMAL = 2;
    const HIGH_LIMIT = 3;
    const LOW_LIMIT = 4;
    const LIFE_SAFETY_ALARM = 5;
}

bacnet_enum! {
    /// BACnet binary present value (Clause 21).
    pub struct BinaryPV(u32);

    const INACTIVE = 0;
    const ACTIVE = 1;
}

bacnet_enum! {
    /// BACnet polarity (Clause 12).
    pub struct Polarity(u32);

    const NORMAL = 0;
    const REVERSE = 1;
}

bacnet_enum! {
    /// BACnet reliability (Clause 12).
    pub struct Reliability(u32);

    const NO_FAULT_DETECTED = 0;
    const NO_SENSOR = 1;
    const OVER_RANGE = 2;
    const UNDER_RANGE = 3;
    const OPEN_LOOP = 4;
    const SHORTED_LOOP = 5;
    const NO_OUTPUT = 6;
    const UNRELIABLE_OTHER = 7;
    const PROCESS_ERROR = 8;
    const MULTI_STATE_FAULT = 9;
    const CONFIGURATION_ERROR = 10;
    // 11: removed from standard
    const COMMUNICATION_FAILURE = 12;
    const MEMBER_FAULT = 13;
    const MONITORED_OBJECT_FAULT = 14;
    const TRIPPED = 15;
    const LAMP_FAILURE = 16;
    const ACTIVATION_FAILURE = 17;
    const RENEW_DHCP_FAILURE = 18;
    const RENEW_FD_REGISTRATION_FAILURE = 19;
    const RESTART_AUTO_NEGOTIATION_FAILURE = 20;
    const RESTART_FAILURE = 21;
    const PROPRIETARY_COMMAND_FAILURE = 22;
    const FAULTS_LISTED = 23;
    const REFERENCED_OBJECT_FAULT = 24;
}

bacnet_enum! {
    /// BACnet device status (Clause 12.11.9).
    pub struct DeviceStatus(u32);

    const OPERATIONAL = 0;
    const OPERATIONAL_READ_ONLY = 1;
    const DOWNLOAD_REQUIRED = 2;
    const DOWNLOAD_IN_PROGRESS = 3;
    const NON_OPERATIONAL = 4;
    const BACKUP_IN_PROGRESS = 5;
}

bacnet_enum! {
    /// BACnet enable/disable (Clause 16.4).
    pub struct EnableDisable(u32);

    const ENABLE = 0;
    /// Deprecated in revision 20; use DISABLE_INITIATION instead.
    const DISABLE = 1;
    const DISABLE_INITIATION = 2;
}

bacnet_enum! {
    /// BACnet reinitialized state of device (Clause 16.5).
    pub struct ReinitializedState(u32);

    const COLDSTART = 0;
    const WARMSTART = 1;
    const START_BACKUP = 2;
    const END_BACKUP = 3;
    const START_RESTORE = 4;
    const END_RESTORE = 5;
    const ABORT_RESTORE = 6;
    const ACTIVATE_CHANGES = 7;
}

bacnet_enum! {
    /// BACnet file access method (Clause 12.12).
    pub struct FileAccessMethod(u32);

    const STREAM_ACCESS = 0;
    const RECORD_ACCESS = 1;
}

bacnet_enum! {
    /// BACnet program state (Clause 12.22).
    pub struct ProgramState(u32);

    const IDLE = 0;
    const LOADING = 1;
    const RUNNING = 2;
    const WAITING = 3;
    const HALTED = 4;
    const UNLOADING = 5;
}

bacnet_enum! {
    /// BACnet program request (Clause 12.22).
    pub struct ProgramChange(u32);

    const READY = 0;
    const LOAD = 1;
    const RUN = 2;
    const HALT = 3;
    const RESTART = 4;
    const UNLOAD = 5;
}

bacnet_enum! {
    /// BACnet action (Clause 12.17).
    pub struct Action(u32);

    const DIRECT = 0;
    const REVERSE = 1;
}

bacnet_enum! {
    /// BACnet event type (Clause 12.12.6).
    pub struct EventType(u32);

    const CHANGE_OF_BITSTRING = 0;
    const CHANGE_OF_STATE = 1;
    const CHANGE_OF_VALUE = 2;
    const COMMAND_FAILURE = 3;
    const FLOATING_LIMIT = 4;
    const OUT_OF_RANGE = 5;
    // 6-7: reserved
    const CHANGE_OF_LIFE_SAFETY = 8;
    const EXTENDED = 9;
    const BUFFER_READY = 10;
    const UNSIGNED_RANGE = 11;
    // 12: reserved
    const ACCESS_EVENT = 13;
    const DOUBLE_OUT_OF_RANGE = 14;
    const SIGNED_OUT_OF_RANGE = 15;
    const UNSIGNED_OUT_OF_RANGE = 16;
    const CHANGE_OF_CHARACTERSTRING = 17;
    const CHANGE_OF_STATUS_FLAGS = 18;
    const CHANGE_OF_RELIABILITY = 19;
    const NONE = 20;
    const CHANGE_OF_DISCRETE_VALUE = 21;
    const CHANGE_OF_TIMER = 22;
}

bacnet_enum! {
    /// BACnet notify type (Clause 12.21).
    pub struct NotifyType(u32);

    const ALARM = 0;
    const EVENT = 1;
    const ACK_NOTIFICATION = 2;
}

bacnet_enum! {
    /// BACnet backup and restore state (Clause 19.1).
    pub struct BackupAndRestoreState(u32);

    const IDLE = 0;
    const PREPARING_FOR_BACKUP = 1;
    const PREPARING_FOR_RESTORE = 2;
    const PERFORMING_A_BACKUP = 3;
    const PERFORMING_A_RESTORE = 4;
}

bacnet_enum! {
    /// BACnet logging type (Clause 12.25.14).
    pub struct LoggingType(u32);

    const POLLED = 0;
    const COV = 1;
    const TRIGGERED = 2;
}
