// ===========================================================================
// Miscellaneous enums
// ===========================================================================

use super::{ConfirmedServiceChoice, UnconfirmedServiceChoice};

bacnet_enum! {
    /// BACnet load control shed state (Clause 12.28).
    pub struct ShedState(u32);

    const SHED_INACTIVE = 0;
    const SHED_REQUEST_PENDING = 1;
    const SHED_COMPLIANT = 2;
    const SHED_NON_COMPLIANT = 3;
}

bacnet_enum! {
    /// BACnet node type for Structured View (Clause 12.29).
    pub struct NodeType(u32);

    const UNKNOWN = 0;
    const SYSTEM = 1;
    const NETWORK = 2;
    const DEVICE = 3;
    const ORGANIZATIONAL = 4;
    const AREA = 5;
    const EQUIPMENT = 6;
    const POINT = 7;
    const COLLECTION = 8;
    const PROPERTY = 9;
    const FUNCTIONAL = 10;
    const OTHER = 11;
    const SUBSYSTEM = 12;
    const BUILDING = 13;
    const FLOOR = 14;
    const SECTION = 15;
    const MODULE = 16;
    const TREE = 17;
    const MEMBER = 18;
    const PROTOCOL = 19;
    const ROOM = 20;
    const ZONE = 21;
}

bacnet_enum! {
    /// BACnet maintenance type (Clause 21); used by life safety point/zone
    /// and access door Maintenance_Required.
    pub struct Maintenance(u32);

    const NONE = 0;
    const PERIODIC_TEST = 1;
    const NEED_SERVICE_OPERATIONAL = 2;
    const NEED_SERVICE_INOPERATIVE = 3;
}

bacnet_enum! {
    /// BACnet subordinate/parent relationship for Structured View
    /// (Clause 21).
    ///
    /// Values above `DEFAULT` come in even/odd forward/reverse pairs
    /// (e.g. `CONTAINS`(2) / `CONTAINED_BY`(3)); proprietary extensions
    /// follow the same pairing so the opposite of a relationship `n > 1` is
    /// always `n ^ 1`.
    pub struct Relationship(u32);

    const UNKNOWN = 0;
    const DEFAULT = 1;
    const CONTAINS = 2;
    const CONTAINED_BY = 3;
    const USES = 4;
    const USED_BY = 5;
    const COMMANDS = 6;
    const COMMANDED_BY = 7;
    const ADJUSTS = 8;
    const ADJUSTED_BY = 9;
    const INGRESS = 10;
    const EGRESS = 11;
    const SUPPLIES_AIR = 12;
    const RECEIVES_AIR = 13;
    const SUPPLIES_HOT_AIR = 14;
    const RECEIVES_HOT_AIR = 15;
    const SUPPLIES_COOL_AIR = 16;
    const RECEIVES_COOL_AIR = 17;
    const SUPPLIES_POWER = 18;
    const RECEIVES_POWER = 19;
    const SUPPLIES_GAS = 20;
    const RECEIVES_GAS = 21;
    const SUPPLIES_WATER = 22;
    const RECEIVES_WATER = 23;
    const SUPPLIES_HOT_WATER = 24;
    const RECEIVES_HOT_WATER = 25;
    const SUPPLIES_COOL_WATER = 26;
    const RECEIVES_COOL_WATER = 27;
    const SUPPLIES_STEAM = 28;
    const RECEIVES_STEAM = 29;
}

bacnet_enum! {
    /// BACnet acknowledgment filter for GetEnrollmentSummary (Clause 13.11.1.1).
    pub struct AcknowledgmentFilter(u32);

    const ALL = 0;
    const ACKED = 1;
    const NOT_ACKED = 2;
}

bacnet_enum! {
    /// Event-state filter for GetEnrollmentSummary (Clause 13.11.1.1).
    pub struct EnrollmentSummaryEventStateFilter(u32);

    const OFFNORMAL = 0;
    const FAULT = 1;
    const NORMAL = 2;
    const ALL = 3;
    const ACTIVE = 4;
}

bacnet_enum! {
    /// Event transition bit positions (Clause 12.11).
    pub struct EventTransitionBits(u8);

    const TO_OFFNORMAL = 0;
    const TO_FAULT = 1;
    const TO_NORMAL = 2;
}

bacnet_enum! {
    /// Bit positions within a `BACnetServicesSupported` bit string (Clause 21).
    pub struct ServiceSupported(u8);

    const ACKNOWLEDGE_ALARM = 0;
    const CONFIRMED_COV_NOTIFICATION = 1;
    const CONFIRMED_EVENT_NOTIFICATION = 2;
    const GET_ALARM_SUMMARY = 3;
    const GET_ENROLLMENT_SUMMARY = 4;
    const SUBSCRIBE_COV = 5;
    const ATOMIC_READ_FILE = 6;
    const ATOMIC_WRITE_FILE = 7;
    const ADD_LIST_ELEMENT = 8;
    const REMOVE_LIST_ELEMENT = 9;
    const CREATE_OBJECT = 10;
    const DELETE_OBJECT = 11;
    const READ_PROPERTY = 12;
    // 13: readPropertyConditional (removed)
    const READ_PROPERTY_MULTIPLE = 14;
    const WRITE_PROPERTY = 15;
    const WRITE_PROPERTY_MULTIPLE = 16;
    const DEVICE_COMMUNICATION_CONTROL = 17;
    const CONFIRMED_PRIVATE_TRANSFER = 18;
    const CONFIRMED_TEXT_MESSAGE = 19;
    const REINITIALIZE_DEVICE = 20;
    const VT_OPEN = 21;
    const VT_CLOSE = 22;
    const VT_DATA = 23;
    // 24: authenticate (removed), 25: requestKey (removed)
    const I_AM = 26;
    const I_HAVE = 27;
    const UNCONFIRMED_COV_NOTIFICATION = 28;
    const UNCONFIRMED_EVENT_NOTIFICATION = 29;
    const UNCONFIRMED_PRIVATE_TRANSFER = 30;
    const UNCONFIRMED_TEXT_MESSAGE = 31;
    const TIME_SYNCHRONIZATION = 32;
    const WHO_HAS = 33;
    const WHO_IS = 34;
    const READ_RANGE = 35;
    const UTC_TIME_SYNCHRONIZATION = 36;
    const LIFE_SAFETY_OPERATION = 37;
    const SUBSCRIBE_COV_PROPERTY = 38;
    const GET_EVENT_INFORMATION = 39;
    const WRITE_GROUP = 40;
    const SUBSCRIBE_COV_PROPERTY_MULTIPLE = 41;
    const CONFIRMED_COV_NOTIFICATION_MULTIPLE = 42;
    const UNCONFIRMED_COV_NOTIFICATION_MULTIPLE = 43;
    const CONFIRMED_AUDIT_NOTIFICATION = 44;
    const AUDIT_LOG_QUERY = 45;
    const UNCONFIRMED_AUDIT_NOTIFICATION = 46;
    const WHO_AM_I = 47;
    const YOU_ARE = 48;
}

impl ServiceSupported {
    /// The `BACnetServicesSupported` bit for a confirmed service choice.
    ///
    /// The choice and bit numberings diverge for every service added after
    /// the original standard (read-range is choice 26 but bit 35,
    /// get-event-information choice 29 but bit 39, …) — always map through
    /// here, never by reusing the choice number as a bit. `None` for choices
    /// with no defined bit (reserved/unassigned values).
    pub fn from_confirmed_choice(choice: ConfirmedServiceChoice) -> Option<Self> {
        Some(match choice.to_raw() {
            0 => Self::ACKNOWLEDGE_ALARM,
            1 => Self::CONFIRMED_COV_NOTIFICATION,
            2 => Self::CONFIRMED_EVENT_NOTIFICATION,
            3 => Self::GET_ALARM_SUMMARY,
            4 => Self::GET_ENROLLMENT_SUMMARY,
            5 => Self::SUBSCRIBE_COV,
            6 => Self::ATOMIC_READ_FILE,
            7 => Self::ATOMIC_WRITE_FILE,
            8 => Self::ADD_LIST_ELEMENT,
            9 => Self::REMOVE_LIST_ELEMENT,
            10 => Self::CREATE_OBJECT,
            11 => Self::DELETE_OBJECT,
            12 => Self::READ_PROPERTY,
            14 => Self::READ_PROPERTY_MULTIPLE,
            15 => Self::WRITE_PROPERTY,
            16 => Self::WRITE_PROPERTY_MULTIPLE,
            17 => Self::DEVICE_COMMUNICATION_CONTROL,
            18 => Self::CONFIRMED_PRIVATE_TRANSFER,
            19 => Self::CONFIRMED_TEXT_MESSAGE,
            20 => Self::REINITIALIZE_DEVICE,
            21 => Self::VT_OPEN,
            22 => Self::VT_CLOSE,
            23 => Self::VT_DATA,
            26 => Self::READ_RANGE,
            27 => Self::LIFE_SAFETY_OPERATION,
            28 => Self::SUBSCRIBE_COV_PROPERTY,
            29 => Self::GET_EVENT_INFORMATION,
            30 => Self::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            31 => Self::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            32 => Self::CONFIRMED_AUDIT_NOTIFICATION,
            33 => Self::AUDIT_LOG_QUERY,
            _ => return None,
        })
    }

    /// The `BACnetServicesSupported` bit for an unconfirmed service choice.
    ///
    /// Every unconfirmed choice diverges from its bit (who-is is choice 8 but
    /// bit 34). `None` for choices with no defined bit.
    pub fn from_unconfirmed_choice(choice: UnconfirmedServiceChoice) -> Option<Self> {
        Some(match choice.to_raw() {
            0 => Self::I_AM,
            1 => Self::I_HAVE,
            2 => Self::UNCONFIRMED_COV_NOTIFICATION,
            3 => Self::UNCONFIRMED_EVENT_NOTIFICATION,
            4 => Self::UNCONFIRMED_PRIVATE_TRANSFER,
            5 => Self::UNCONFIRMED_TEXT_MESSAGE,
            6 => Self::TIME_SYNCHRONIZATION,
            7 => Self::WHO_HAS,
            8 => Self::WHO_IS,
            9 => Self::UTC_TIME_SYNCHRONIZATION,
            10 => Self::WRITE_GROUP,
            11 => Self::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
            12 => Self::UNCONFIRMED_AUDIT_NOTIFICATION,
            13 => Self::WHO_AM_I,
            14 => Self::YOU_ARE,
            _ => return None,
        })
    }
}

bacnet_enum! {
    /// BACnet message priority for TextMessage services (Clause 16.5).
    pub struct MessagePriority(u32);

    const NORMAL = 0;
    const URGENT = 1;
}

bacnet_enum! {
    /// BACnet virtual terminal class (Clause 17.1).
    pub struct VTClass(u32);

    const DEFAULT_TERMINAL = 0;
    const ANSI_X3_64 = 1;
    const DEC_VT52 = 2;
    const DEC_VT100 = 3;
    const DEC_VT220 = 4;
    const HP_700_94 = 5;
    const IBM_3130 = 6;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_choice_to_bit_divergent_pairs() {
        // The pairs where choice != bit — the exact confusions that produced
        // the old hardcoded Protocol_Services_Supported constant (#192).
        let confirmed = [
            (26, ServiceSupported::READ_RANGE, 35),
            (27, ServiceSupported::LIFE_SAFETY_OPERATION, 37),
            (28, ServiceSupported::SUBSCRIBE_COV_PROPERTY, 38),
            (29, ServiceSupported::GET_EVENT_INFORMATION, 39),
            (30, ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE, 41),
            (
                31,
                ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                42,
            ),
            (32, ServiceSupported::CONFIRMED_AUDIT_NOTIFICATION, 44),
            (33, ServiceSupported::AUDIT_LOG_QUERY, 45),
        ];
        for (choice, expected, bit) in confirmed {
            let got =
                ServiceSupported::from_confirmed_choice(ConfirmedServiceChoice::from_raw(choice))
                    .unwrap();
            assert_eq!(got, expected);
            assert_eq!(got.to_raw(), bit);
        }

        let unconfirmed = [
            (0, ServiceSupported::I_AM, 26),
            (5, ServiceSupported::UNCONFIRMED_TEXT_MESSAGE, 31),
            (6, ServiceSupported::TIME_SYNCHRONIZATION, 32),
            (7, ServiceSupported::WHO_HAS, 33),
            (8, ServiceSupported::WHO_IS, 34),
            (9, ServiceSupported::UTC_TIME_SYNCHRONIZATION, 36),
            (10, ServiceSupported::WRITE_GROUP, 40),
            (14, ServiceSupported::YOU_ARE, 48),
        ];
        for (choice, expected, bit) in unconfirmed {
            let got = ServiceSupported::from_unconfirmed_choice(
                UnconfirmedServiceChoice::from_raw(choice),
            )
            .unwrap();
            assert_eq!(got, expected);
            assert_eq!(got.to_raw(), bit);
        }
    }

    #[test]
    fn service_choice_identity_range_and_reserved() {
        // Choices 0..=23 (minus reserved 13) map bit == choice.
        for c in (0..=23u8).filter(|c| *c != 13) {
            let bit = ServiceSupported::from_confirmed_choice(ConfirmedServiceChoice::from_raw(c))
                .unwrap()
                .to_raw();
            assert_eq!(bit, c, "choice {c} should be the identity mapping");
        }
        // Reserved/unassigned choices have no bit.
        assert!(
            ServiceSupported::from_confirmed_choice(ConfirmedServiceChoice::from_raw(13)).is_none()
        );
        assert!(
            ServiceSupported::from_confirmed_choice(ConfirmedServiceChoice::from_raw(34)).is_none()
        );
        assert!(
            ServiceSupported::from_unconfirmed_choice(UnconfirmedServiceChoice::from_raw(15))
                .is_none()
        );
    }
}
