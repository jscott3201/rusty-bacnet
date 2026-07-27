mod priority;
mod properties;
mod recipient_list;
mod recipients;

use super::*;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;

fn make_time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn make_dest_device(device_instance: u32) -> BACnetDestination {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
    BACnetDestination {
        valid_days: 0b0111_1111, // all days
        from_time: make_time(0, 0),
        to_time: make_time(23, 59),
        recipient: BACnetRecipient::Device(dev_oid),
        process_identifier: 1,
        issue_confirmed_notifications: true,
        transitions: 0b0000_0111, // all transitions
    }
}
