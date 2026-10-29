//! Channel's RPM selectors follow its Table 12-62 rows (#1151, #1264).
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{channel::ChannelObject, traits::BACnetObject};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;

#[test]
fn rpm_channel_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::DESCRIPTION,
        P::PRESENT_VALUE,
        P::LAST_PRIORITY,
        P::WRITE_STATUS,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        P::EXECUTION_DELAY,
        P::ALLOW_GROUP_DELAY_INHIBIT,
        P::CHANNEL_NUMBER,
        P::CONTROL_GROUPS,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::LAST_PRIORITY,
        P::WRITE_STATUS,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        P::CHANNEL_NUMBER,
        P::CONTROL_GROUPS,
    ];
    let optional = [
        P::DESCRIPTION,
        P::RELIABILITY,
        P::EXECUTION_DELAY,
        P::ALLOW_GROUP_DELAY_INHIBIT,
    ];
    for configured in [false, true] {
        let mut object = ChannelObject::new(7, "CH-7", 11).unwrap();
        if configured {
            object
                .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
                    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                    P::PRESENT_VALUE.to_raw(),
                )])
                .unwrap();
            object.set_execution_delay(vec![250]).unwrap();
            object.set_control_groups(vec![27, 14]).unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        for (selector, expected) in [
            (P::ALL, all.as_slice()),
            (P::REQUIRED, required.as_slice()),
            (P::OPTIONAL, optional.as_slice()),
            (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
        ] {
            assert_rpm_selector_bytes(&db, oid, selector, expected);
        }
    }
}
