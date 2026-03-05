//! NotificationClass object per ASHRAE 135-2020 Clause 12.31.

use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags, Time};
use bacnet_types::MacAddr;
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::database::ObjectDatabase;
use crate::event::EventTransition;
use crate::traits::BACnetObject;

/// BACnet NotificationClass object.
///
/// Stores notification routing configuration: which priorities, acknowledgement
/// requirements, and recipient destinations apply to event notifications
/// referencing this class number.
pub struct NotificationClass {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// The notification class number.
    pub notification_class: u32,
    /// Priority: [TO_OFFNORMAL, TO_FAULT, TO_NORMAL]. Default [255, 255, 255].
    pub priority: [u8; 3],
    /// Ack required: [TO_OFFNORMAL, TO_FAULT, TO_NORMAL]. Default [false, false, false].
    pub ack_required: [bool; 3],
    /// Recipient list.
    pub recipient_list: Vec<BACnetDestination>,
}

impl NotificationClass {
    /// Create a new NotificationClass object.
    ///
    /// The `notification_class` number defaults to the instance number.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            notification_class: instance,
            priority: [255, 255, 255],
            ack_required: [false, false, false],
            recipient_list: Vec::new(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Add a destination to the recipient list.
    pub fn add_destination(&mut self, dest: BACnetDestination) {
        self.recipient_list.push(dest);
    }
}

impl BACnetObject for NotificationClass {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_CLASS.to_raw(),
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(0)) // normal
            }
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => {
                Ok(PropertyValue::Unsigned(self.notification_class as u64))
            }
            p if p == PropertyIdentifier::PRIORITY => match array_index {
                Some(0) => Ok(PropertyValue::Unsigned(3)),
                Some(idx) if (1..=3).contains(&idx) => Ok(PropertyValue::Unsigned(
                    self.priority[(idx - 1) as usize] as u64,
                )),
                None => Ok(PropertyValue::List(vec![
                    PropertyValue::Unsigned(self.priority[0] as u64),
                    PropertyValue::Unsigned(self.priority[1] as u64),
                    PropertyValue::Unsigned(self.priority[2] as u64),
                ])),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ACK_REQUIRED => {
                // 3-bit bitstring: bit 0=TO_OFFNORMAL, bit 1=TO_FAULT, bit 2=TO_NORMAL
                let mut byte: u8 = 0;
                if self.ack_required[0] {
                    byte |= 0x80;
                } // bit 0 in MSB
                if self.ack_required[1] {
                    byte |= 0x40;
                } // bit 1
                if self.ack_required[2] {
                    byte |= 0x20;
                } // bit 2
                Ok(PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![byte],
                })
            }
            p if p == PropertyIdentifier::RECIPIENT_LIST => Ok(PropertyValue::List(
                self.recipient_list
                    .iter()
                    .map(|dest| {
                        PropertyValue::List(vec![
                            // valid_days as bitstring (7 bits used, 1 unused)
                            PropertyValue::BitString {
                                unused_bits: 1,
                                data: vec![dest.valid_days << 1],
                            },
                            PropertyValue::Time(dest.from_time),
                            PropertyValue::Time(dest.to_time),
                            // recipient
                            match &dest.recipient {
                                BACnetRecipient::Device(oid) => {
                                    PropertyValue::ObjectIdentifier(*oid)
                                }
                                BACnetRecipient::Address(addr) => {
                                    PropertyValue::OctetString(addr.mac_address.to_vec())
                                }
                            },
                            PropertyValue::Unsigned(dest.process_identifier as u64),
                            PropertyValue::Boolean(dest.issue_confirmed_notifications),
                            // transitions as bitstring (3 bits used, 5 unused)
                            PropertyValue::BitString {
                                unused_bits: 5,
                                data: vec![dest.transitions << 5],
                            },
                        ])
                    })
                    .collect(),
            )),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::NOTIFICATION_CLASS {
            if let PropertyValue::Unsigned(v) = value {
                self.notification_class = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::RECIPIENT_LIST {
            if let PropertyValue::List(entries) = value {
                let mut new_list = Vec::with_capacity(entries.len());
                for entry in entries {
                    if let PropertyValue::List(fields) = entry {
                        if fields.len() < 7 {
                            return Err(common::invalid_data_type_error());
                        }
                        // [0] valid_days: BitString (7 bits, 1 unused)
                        let valid_days = match &fields[0] {
                            PropertyValue::BitString { data, .. } if !data.is_empty() => {
                                data[0] >> 1
                            }
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [1] from_time
                        let from_time = match fields[1] {
                            PropertyValue::Time(t) => t,
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [2] to_time
                        let to_time = match fields[2] {
                            PropertyValue::Time(t) => t,
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [3] recipient: ObjectIdentifier (Device) or OctetString (Address)
                        let recipient = match &fields[3] {
                            PropertyValue::ObjectIdentifier(oid) => BACnetRecipient::Device(*oid),
                            PropertyValue::OctetString(mac) => {
                                BACnetRecipient::Address(BACnetAddress {
                                    network_number: 0,
                                    mac_address: MacAddr::from_slice(mac),
                                })
                            }
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [4] process_identifier
                        let process_identifier = match fields[4] {
                            PropertyValue::Unsigned(v) => common::u64_to_u32(v)?,
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [5] issue_confirmed_notifications
                        let issue_confirmed_notifications = match fields[5] {
                            PropertyValue::Boolean(b) => b,
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        // [6] transitions: BitString (3 bits, 5 unused)
                        let transitions = match &fields[6] {
                            PropertyValue::BitString { data, .. } if !data.is_empty() => {
                                data[0] >> 5
                            }
                            _ => return Err(common::invalid_data_type_error()),
                        };
                        new_list.push(BACnetDestination {
                            valid_days,
                            from_time,
                            to_time,
                            recipient,
                            process_identifier,
                            issue_confirmed_notifications,
                            transitions,
                        });
                    } else {
                        return Err(common::invalid_data_type_error());
                    }
                }
                self.recipient_list = new_list;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::PRIORITY,
            PropertyIdentifier::ACK_REQUIRED,
            PropertyIdentifier::RECIPIENT_LIST,
        ];
        Cow::Borrowed(PROPS)
    }
}

/// Convert a `Time` to centiseconds (hundredths of a second since midnight).
fn time_to_centiseconds(t: &Time) -> u32 {
    let h = if t.hour == Time::UNSPECIFIED {
        0
    } else {
        t.hour as u32
    };
    let m = if t.minute == Time::UNSPECIFIED {
        0
    } else {
        t.minute as u32
    };
    let s = if t.second == Time::UNSPECIFIED {
        0
    } else {
        t.second as u32
    };
    let cs = if t.hundredths == Time::UNSPECIFIED {
        0
    } else {
        t.hundredths as u32
    };
    h * 360_000 + m * 6_000 + s * 100 + cs
}

/// Check if `current` falls within the `[from, to]` time window.
///
/// If either bound has an unspecified hour (0xFF), the window is treated as "all day".
fn time_in_window(current: &Time, from: &Time, to: &Time) -> bool {
    if from.hour == Time::UNSPECIFIED || to.hour == Time::UNSPECIFIED {
        return true;
    }
    let cur = time_to_centiseconds(current);
    let from_cs = time_to_centiseconds(from);
    let to_cs = time_to_centiseconds(to);
    cur >= from_cs && cur <= to_cs
}

/// Get notification recipients for a given notification class number and transition.
///
/// Looks up the `NotificationClass` object whose `Notification_Class` property equals
/// `notification_class`, then filters its `Recipient_List` by day, time, and transition.
///
/// # Parameters
/// - `db`: the object database containing NotificationClass objects
/// - `notification_class`: the notification class number to look up
/// - `transition`: which event transition to filter for
/// - `today_bit`: bitmask for today's day of week in `valid_days`
///   (bit 0 = Sunday, bit 1 = Monday, …, bit 6 = Saturday)
/// - `current_time`: the current local time for time-window filtering
///
/// Returns `(recipient, process_identifier, issue_confirmed_notifications)` tuples.
/// Returns an empty `Vec` if no matching NotificationClass is found or no recipients match.
pub fn get_notification_recipients(
    db: &ObjectDatabase,
    notification_class: u32,
    transition: EventTransition,
    today_bit: u8,
    current_time: &Time,
) -> Vec<(BACnetRecipient, u32, bool)> {
    // Try direct OID lookup first (instance == notification_class is the common case)
    let recipient_list_val = if let Ok(nc_oid) =
        ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, notification_class)
    {
        if let Some(obj) = db.get(&nc_oid) {
            match obj.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None) {
                Ok(PropertyValue::Unsigned(n)) if n as u32 == notification_class => obj
                    .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
                    .ok(),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    // Fall back to scanning all NotificationClass objects
    let recipient_list_val = recipient_list_val.or_else(|| {
        db.find_by_type(ObjectType::NOTIFICATION_CLASS)
            .iter()
            .find_map(|oid| {
                let obj = db.get(oid)?;
                match obj.read_property(PropertyIdentifier::NOTIFICATION_CLASS, None) {
                    Ok(PropertyValue::Unsigned(n)) if n as u32 == notification_class => obj
                        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
                        .ok(),
                    _ => None,
                }
            })
    });

    let recipient_list_val = match recipient_list_val {
        Some(v) => v,
        None => return Vec::new(),
    };

    filter_recipient_list(&recipient_list_val, transition, today_bit, current_time)
}

/// Filter an encoded `RECIPIENT_LIST` property value by day, time, and transition.
///
/// Parses `PropertyValue::List` entries (as returned by `read_property(RECIPIENT_LIST)`)
/// and returns only those recipients matching the given filters.
pub fn filter_recipient_list(
    recipient_list_value: &PropertyValue,
    transition: EventTransition,
    today_bit: u8,
    current_time: &Time,
) -> Vec<(BACnetRecipient, u32, bool)> {
    let entries = match recipient_list_value {
        PropertyValue::List(l) => l,
        _ => return Vec::new(),
    };

    let transition_mask = transition.bit_mask();
    let mut result = Vec::new();

    for entry in entries {
        let fields = match entry {
            PropertyValue::List(f) if f.len() >= 7 => f,
            _ => continue,
        };

        // [0] valid_days bitstring
        let valid_days = match &fields[0] {
            PropertyValue::BitString { data, .. } if !data.is_empty() => data[0] >> 1,
            _ => continue,
        };
        if valid_days & today_bit == 0 {
            continue;
        }

        // [1] from_time, [2] to_time
        let from_time = match &fields[1] {
            PropertyValue::Time(t) => t,
            _ => continue,
        };
        let to_time = match &fields[2] {
            PropertyValue::Time(t) => t,
            _ => continue,
        };
        if !time_in_window(current_time, from_time, to_time) {
            continue;
        }

        // [6] transitions bitstring
        let transitions = match &fields[6] {
            PropertyValue::BitString { data, .. } if !data.is_empty() => data[0] >> 5,
            _ => continue,
        };
        if transitions & transition_mask == 0 {
            continue;
        }

        // [3] recipient
        let recipient = match &fields[3] {
            PropertyValue::ObjectIdentifier(oid) => BACnetRecipient::Device(*oid),
            PropertyValue::OctetString(mac) => BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(mac),
            }),
            _ => continue,
        };

        // [4] process_identifier
        let process_id = match &fields[4] {
            PropertyValue::Unsigned(v) => *v as u32,
            _ => continue,
        };

        // [5] issue_confirmed_notifications
        let confirmed = match &fields[5] {
            PropertyValue::Boolean(b) => *b,
            _ => continue,
        };

        result.push((recipient, process_id, confirmed));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
    use bacnet_types::primitives::Time;
    use bacnet_types::MacAddr;

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

    #[test]
    fn object_type_is_notification_class() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        assert_eq!(
            nc.object_identifier().object_type(),
            ObjectType::NOTIFICATION_CLASS
        );
        assert_eq!(nc.object_identifier().instance_number(), 1);
    }

    #[test]
    fn read_notification_class_number() {
        let nc = NotificationClass::new(42, "NC-42").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
            .unwrap();
        if let PropertyValue::Unsigned(n) = val {
            assert_eq!(n, 42);
        } else {
            panic!("Expected Unsigned");
        }
    }

    #[test]
    fn read_priority_array_index() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        // Index 0 = array length
        let len = nc
            .read_property(PropertyIdentifier::PRIORITY, Some(0))
            .unwrap();
        if let PropertyValue::Unsigned(n) = len {
            assert_eq!(n, 3);
        } else {
            panic!("Expected Unsigned");
        }

        // Index 1 = TO_OFFNORMAL priority (default 255)
        let p1 = nc
            .read_property(PropertyIdentifier::PRIORITY, Some(1))
            .unwrap();
        if let PropertyValue::Unsigned(n) = p1 {
            assert_eq!(n, 255);
        } else {
            panic!("Expected Unsigned");
        }
    }

    #[test]
    fn read_priority_all() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::PRIORITY, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], PropertyValue::Unsigned(255));
            assert_eq!(items[1], PropertyValue::Unsigned(255));
            assert_eq!(items[2], PropertyValue::Unsigned(255));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn read_priority_invalid_index() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let result = nc.read_property(PropertyIdentifier::PRIORITY, Some(4));
        assert!(result.is_err());
    }

    #[test]
    fn read_object_name() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        if let PropertyValue::CharacterString(s) = val {
            assert_eq!(s, "NC-1");
        } else {
            panic!("Expected CharacterString");
        }
    }

    #[test]
    fn write_notification_class_number() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        nc.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(99),
            None,
        )
        .unwrap();
        assert_eq!(nc.notification_class, 99);
    }

    #[test]
    fn write_notification_class_wrong_type() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        let result = nc.write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn property_list_contains_recipient_list() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let props = nc.property_list();
        assert!(props.contains(&PropertyIdentifier::NOTIFICATION_CLASS));
        assert!(props.contains(&PropertyIdentifier::PRIORITY));
        assert!(props.contains(&PropertyIdentifier::ACK_REQUIRED));
        assert!(props.contains(&PropertyIdentifier::RECIPIENT_LIST));
    }

    #[test]
    fn read_ack_required_default() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::ACK_REQUIRED, None)
            .unwrap();
        if let PropertyValue::BitString { unused_bits, data } = val {
            assert_eq!(unused_bits, 5);
            assert_eq!(data, vec![0]); // all false
        } else {
            panic!("Expected BitString");
        }
    }

    #[test]
    fn read_recipient_list_empty() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert!(items.is_empty());
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn add_destination_device_and_read_back() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        nc.add_destination(make_dest_device(99));

        let val = nc
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        let PropertyValue::List(outer) = val else {
            panic!("Expected outer List");
        };
        assert_eq!(outer.len(), 1);

        let PropertyValue::List(fields) = &outer[0] else {
            panic!("Expected inner List");
        };
        // 7 fields: valid_days, from_time, to_time, recipient, process_id, confirmed, transitions
        assert_eq!(fields.len(), 7);

        // valid_days bitstring: all days = 0b0111_1111 << 1 = 0b1111_1110 = 0xFE
        assert_eq!(
            fields[0],
            PropertyValue::BitString {
                unused_bits: 1,
                data: vec![0b1111_1110],
            }
        );

        // from_time
        assert_eq!(fields[1], PropertyValue::Time(make_time(0, 0)));

        // to_time
        assert_eq!(fields[2], PropertyValue::Time(make_time(23, 59)));

        // recipient = Device OID for instance 99
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
        assert_eq!(fields[3], PropertyValue::ObjectIdentifier(dev_oid));

        // process_identifier
        assert_eq!(fields[4], PropertyValue::Unsigned(1));

        // issue_confirmed_notifications
        assert_eq!(fields[5], PropertyValue::Boolean(true));

        // transitions: all = 0b0000_0111 << 5 = 0b1110_0000 = 0xE0
        assert_eq!(
            fields[6],
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0b1110_0000],
            }
        );
    }

    #[test]
    fn add_destination_address_variant() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        let mac = MacAddr::from_slice(&[192u8, 168, 1, 100, 0xBA, 0xC0]);
        let dest = BACnetDestination {
            valid_days: 0b0011_1110, // Mon–Fri
            from_time: make_time(8, 0),
            to_time: make_time(17, 0),
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: mac.clone(),
            }),
            process_identifier: 42,
            issue_confirmed_notifications: false,
            transitions: 0b0000_0001, // TO_OFFNORMAL only
        };
        nc.add_destination(dest);

        let val = nc
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        let PropertyValue::List(outer) = val else {
            panic!("Expected outer List");
        };
        assert_eq!(outer.len(), 1);

        let PropertyValue::List(fields) = &outer[0] else {
            panic!("Expected inner List");
        };

        // recipient = OctetString of mac_address
        assert_eq!(fields[3], PropertyValue::OctetString(mac.to_vec()));

        // process_identifier = 42
        assert_eq!(fields[4], PropertyValue::Unsigned(42));

        // issue_confirmed = false
        assert_eq!(fields[5], PropertyValue::Boolean(false));

        // transitions: bit 0 only = 0b0000_0001 << 5 = 0b0010_0000 = 0x20
        assert_eq!(
            fields[6],
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0b0010_0000],
            }
        );
    }

    #[test]
    fn add_multiple_destinations() {
        let mut nc = NotificationClass::new(5, "NC-5").unwrap();
        nc.add_destination(make_dest_device(100));
        nc.add_destination(make_dest_device(200));
        nc.add_destination(make_dest_device(300));

        let val = nc
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        let PropertyValue::List(outer) = val else {
            panic!("Expected List");
        };
        assert_eq!(outer.len(), 3);
    }

    #[test]
    fn write_recipient_list_clears_existing() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        nc.add_destination(make_dest_device(10));
        nc.add_destination(make_dest_device(20));
        assert_eq!(nc.recipient_list.len(), 2);

        // Write an empty list — should clear
        nc.write_property(
            PropertyIdentifier::RECIPIENT_LIST,
            None,
            PropertyValue::List(vec![]),
            None,
        )
        .unwrap();
        assert!(nc.recipient_list.is_empty());
    }

    #[test]
    fn write_recipient_list_wrong_type_denied() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        let result = nc.write_property(
            PropertyIdentifier::RECIPIENT_LIST,
            None,
            PropertyValue::Unsigned(0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn write_recipient_list_round_trip() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        nc.add_destination(make_dest_device(10));
        // Read the encoded list, then write it back
        let encoded = nc
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        nc.write_property(PropertyIdentifier::RECIPIENT_LIST, None, encoded, None)
            .unwrap();
        assert_eq!(nc.recipient_list.len(), 1);
        assert_eq!(nc.recipient_list[0].process_identifier, 1);
    }

    #[test]
    fn read_event_state_default() {
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        let val = nc
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // normal
    }

    #[test]
    fn write_out_of_service() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        nc.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let val = nc
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn write_unknown_property_denied() {
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        let result = nc.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(1.0),
            None,
        );
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // get_notification_recipients tests
    // -----------------------------------------------------------------------

    fn make_dest(
        device_instance: u32,
        valid_days: u8,
        from: Time,
        to: Time,
        confirmed: bool,
        transitions: u8,
    ) -> BACnetDestination {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
        BACnetDestination {
            valid_days,
            from_time: from,
            to_time: to,
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: device_instance,
            issue_confirmed_notifications: confirmed,
            transitions,
        }
    }

    #[test]
    fn get_recipients_filters_by_transition() {
        let mut db = ObjectDatabase::new();
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();

        // Recipient 1: only TO_OFFNORMAL (bit 0)
        nc.add_destination(make_dest(
            10,
            0b0111_1111,
            make_time(0, 0),
            make_time(23, 59),
            false,
            0b0000_0001,
        ));
        // Recipient 2: only TO_NORMAL (bit 2)
        nc.add_destination(make_dest(
            20,
            0b0111_1111,
            make_time(0, 0),
            make_time(23, 59),
            true,
            0b0000_0100,
        ));
        // Recipient 3: all transitions
        nc.add_destination(make_dest(
            30,
            0b0111_1111,
            make_time(0, 0),
            make_time(23, 59),
            false,
            0b0000_0111,
        ));
        db.add(Box::new(nc)).unwrap();

        let now = make_time(12, 0);
        let monday_bit = 0x02; // bit 1 = Monday

        // TO_OFFNORMAL should match recipients 1 and 3
        let r = get_notification_recipients(&db, 1, EventTransition::ToOffnormal, monday_bit, &now);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, 10); // process_id
        assert_eq!(r[1].1, 30);

        // TO_NORMAL should match recipients 2 and 3
        let r = get_notification_recipients(&db, 1, EventTransition::ToNormal, monday_bit, &now);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, 20);
        assert!(r[0].2); // recipient 2 is confirmed
        assert_eq!(r[1].1, 30);

        // TO_FAULT should match only recipient 3
        let r = get_notification_recipients(&db, 1, EventTransition::ToFault, monday_bit, &now);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, 30);
    }

    #[test]
    fn get_recipients_filters_by_day() {
        let mut db = ObjectDatabase::new();
        let mut nc = NotificationClass::new(2, "NC-2").unwrap();

        // Recipient valid Mon-Fri only (bits 1-5 = 0b0011_1110)
        nc.add_destination(make_dest(
            10,
            0b0011_1110,
            make_time(0, 0),
            make_time(23, 59),
            false,
            0b0000_0111,
        ));
        db.add(Box::new(nc)).unwrap();

        let now = make_time(12, 0);

        // Monday (bit 1) — should match
        let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x02, &now);
        assert_eq!(r.len(), 1);

        // Sunday (bit 0) — should NOT match
        let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x01, &now);
        assert!(r.is_empty());

        // Saturday (bit 6) — should NOT match
        let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x40, &now);
        assert!(r.is_empty());
    }

    #[test]
    fn get_recipients_filters_by_time_window() {
        let mut db = ObjectDatabase::new();
        let mut nc = NotificationClass::new(3, "NC-3").unwrap();

        // Recipient valid 08:00–17:00
        nc.add_destination(make_dest(
            10,
            0b0111_1111,
            make_time(8, 0),
            make_time(17, 0),
            false,
            0b0000_0111,
        ));
        db.add(Box::new(nc)).unwrap();

        let monday_bit = 0x02;

        // 12:00 — inside window
        let r = get_notification_recipients(
            &db,
            3,
            EventTransition::ToOffnormal,
            monday_bit,
            &make_time(12, 0),
        );
        assert_eq!(r.len(), 1);

        // 07:00 — before window
        let r = get_notification_recipients(
            &db,
            3,
            EventTransition::ToOffnormal,
            monday_bit,
            &make_time(7, 0),
        );
        assert!(r.is_empty());

        // 18:00 — after window
        let r = get_notification_recipients(
            &db,
            3,
            EventTransition::ToOffnormal,
            monday_bit,
            &make_time(18, 0),
        );
        assert!(r.is_empty());
    }

    #[test]
    fn get_recipients_returns_empty_for_missing_class() {
        let db = ObjectDatabase::new();
        let r = get_notification_recipients(
            &db,
            99,
            EventTransition::ToOffnormal,
            0x02,
            &make_time(12, 0),
        );
        assert!(r.is_empty());
    }

    #[test]
    fn get_recipients_returns_empty_for_empty_list() {
        let mut db = ObjectDatabase::new();
        let nc = NotificationClass::new(1, "NC-1").unwrap();
        db.add(Box::new(nc)).unwrap();

        let r = get_notification_recipients(
            &db,
            1,
            EventTransition::ToOffnormal,
            0x02,
            &make_time(12, 0),
        );
        assert!(r.is_empty());
    }

    #[test]
    fn event_state_change_transition_mapping() {
        use crate::event::EventStateChange;
        use bacnet_types::enums::EventState;

        let to_normal = EventStateChange {
            from: EventState::HIGH_LIMIT,
            to: EventState::NORMAL,
        };
        assert_eq!(to_normal.transition(), EventTransition::ToNormal);

        let to_fault = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::FAULT,
        };
        assert_eq!(to_fault.transition(), EventTransition::ToFault);

        let to_high = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        };
        assert_eq!(to_high.transition(), EventTransition::ToOffnormal);

        let to_low = EventStateChange {
            from: EventState::NORMAL,
            to: EventState::LOW_LIMIT,
        };
        assert_eq!(to_low.transition(), EventTransition::ToOffnormal);
    }
}
