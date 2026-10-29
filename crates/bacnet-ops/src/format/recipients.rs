//! Notification and subscription elements as text: Recipient_List
//! destinations, recipients, COV subscriptions, address bindings and
//! property states.

use bacnet_encoding::constructed::{decode_cov_subscription, decode_destination, decode_recipient};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVSubscription, BACnetDestination, BACnetPropertyStates, BACnetRecipient,
};
use bacnet_types::enums::{
    AccessEvent, AccessZoneOccupancyState, BinaryPV, DeviceStatus, DoorAlarmState,
    DoorSecuredStatus, DoorStatus, EngineeringUnits, EventState, EventType, FileAccessMethod,
    LifeSafetyMode, LifeSafetyOperation, LifeSafetyState, LockStatus, Maintenance, NodeType,
    NotifyType, Polarity, ProgramChange, ProgramError, ProgramState, Reliability, RestartReason,
    SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::ValueFormat;
use super::constructed::reference;
use super::primitive::{hex, time_text, weekday};
use crate::label::kebab;

/// `network 5 address 0x0a000001bac0`; an empty MAC, a broadcast on
/// that network, as `network 5 broadcast`.
pub(super) fn address_text(address: &BACnetAddress) -> String {
    network_mac(address.network_number, &address.mac_address)
}

/// A network number and MAC, as [`address_text`] writes them.
fn network_mac(network: u16, mac: &[u8]) -> String {
    if mac.is_empty() {
        format!("network {network} broadcast")
    } else {
        format!("network {network} address 0x{}", hex(mac, ""))
    }
}

/// A recipient: `device 1234`, or its address.
fn recipient_text(recipient: &BACnetRecipient) -> String {
    match recipient {
        BACnetRecipient::Device(device) => format!("device {}", device.instance_number()),
        BACnetRecipient::Address(address) => address_text(address),
    }
}

/// The days of a mask as runs: `Mon-Fri`, `Mon, Wed, Sat-Sun`, `every day`
/// or `no days`.
fn days_text(days: DaysOfWeek) -> String {
    let set: Vec<bool> = (0..7).map(|day| days.bits() & (1 << day) != 0).collect();
    match set.iter().filter(|on| **on).count() {
        0 => return "no days".into(),
        7 => return "every day".into(),
        _ => {}
    }
    let name = |day: usize| weekday(day as u8 + 1).unwrap_or("?");
    let mut runs = Vec::new();
    let mut day = 0;
    while day < 7 {
        if !set[day] {
            day += 1;
            continue;
        }
        let start = day;
        while day + 1 < 7 && set[day + 1] {
            day += 1;
        }
        runs.push(if day == start {
            name(start).to_string()
        } else {
            format!("{}-{}", name(start), name(day))
        });
        day += 1;
    }
    runs.join(", ")
}

/// Event transitions by name, `no transitions` when none is set.
fn transitions_text(transitions: EventTransitionBits) -> String {
    if transitions.is_empty() {
        "no transitions".into()
    } else {
        kebab(transitions)
    }
}

fn confirmed_text(confirmed: bool) -> &'static str {
    if confirmed {
        "confirmed"
    } else {
        "unconfirmed"
    }
}

/// One Recipient_List destination: `device 1234 process 7: Mon-Fri
/// 08:00-17:00, confirmed, to-offnormal | to-fault | to-normal`.
fn destination_text(destination: &BACnetDestination) -> String {
    format!(
        "{} process {}: {} {}-{}, {}, {}",
        recipient_text(&destination.recipient),
        destination.process_identifier,
        days_text(destination.valid_days),
        time_text(&destination.from_time),
        time_text(&destination.to_time),
        confirmed_text(destination.issue_confirmed_notifications),
        transitions_text(destination.transitions),
    )
}

pub(super) fn destination(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_destination(octets, offset).map(|(d, end)| (destination_text(&d), end))
}

pub(super) fn recipient(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_recipient(octets, offset).map(|(r, end)| (recipient_text(&r), end))
}

/// One Active_COV_Subscriptions element: `device 1234 process 7:
/// analog-input 1 present-value, unconfirmed, 120 s left, increment 0.5`.
fn cov_subscription_text(format: &ValueFormat<'_>, subscription: &BACnetCOVSubscription) -> String {
    let monitored = &subscription.monitored_property_reference;
    let mut text = format!(
        "{} process {}: {}, {}, ",
        recipient_text(&subscription.recipient.recipient),
        subscription.recipient.process_identifier,
        reference(
            monitored.object_identifier,
            monitored.property_identifier,
            monitored.property_array_index
        ),
        confirmed_text(subscription.issue_confirmed_notifications),
    );
    match subscription.time_remaining {
        0 => text.push_str("no expiry"),
        seconds => text.push_str(&format!("{seconds} s left")),
    }
    if let Some(increment) = subscription.cov_increment {
        text.push_str(&format!(", increment {}", format.real(increment)));
    }
    text
}

pub(super) fn cov_subscription(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_cov_subscription(octets, offset).map(|(s, end)| (cov_subscription_text(format, &s), end))
}

/// One Device_Address_Binding element, the Device's identifier then its
/// network number and MAC, each application tagged: `device 1234 at
/// network 0 address 0x0a000001bac0`.
pub(super) fn address_binding(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    let (device, network, mac, end) = decode_address_binding(octets, offset)?;
    let text = format!(
        "device {} at {}",
        device.instance_number(),
        network_mac(network, &mac)
    );
    Ok((text, end))
}

/// The Device, network number and MAC of the binding at `offset`, and the
/// offset past it.
fn decode_address_binding(
    octets: &[u8],
    offset: usize,
) -> Result<(ObjectIdentifier, u16, Vec<u8>, usize), Error> {
    let (PropertyValue::ObjectIdentifier(device), at) = decode_application_value(octets, offset)?
    else {
        return Err(Error::decoding(offset, "expected the binding's Device"));
    };
    let (PropertyValue::Unsigned(network), mac_at) = decode_application_value(octets, at)? else {
        return Err(Error::decoding(at, "expected the binding's network number"));
    };
    let network = u16::try_from(network)
        .map_err(|_| Error::decoding(at, "the binding's network number is over 65535"))?;
    let (PropertyValue::OctetString(mac), end) = decode_application_value(octets, mac_at)? else {
        return Err(Error::decoding(mac_at, "expected the binding's MAC"));
    };
    Ok((device, network, mac, end))
}

/// A property state by name where its enumeration is known: `high-limit`,
/// `active`, `degrees-fahrenheit`; any other as its choice and value.
pub(super) fn property_state_text(state: &BACnetPropertyStates) -> String {
    use BACnetPropertyStates as S;
    match state {
        S::BooleanValue(value) => value.to_string(),
        S::UnsignedValue(value) => value.to_string(),
        S::IntegerValue(value) => value.to_string(),
        S::BinaryValue(v) => kebab(BinaryPV::from_raw(*v)),
        S::EventType(v) => kebab(EventType::from_raw(*v)),
        S::Polarity(v) => kebab(Polarity::from_raw(*v)),
        S::ProgramChange(v) => kebab(ProgramChange::from_raw(*v)),
        S::ProgramState(v) => kebab(ProgramState::from_raw(*v)),
        S::ReasonForHalt(v) => kebab(ProgramError::from_raw(*v)),
        S::Reliability(v) => kebab(Reliability::from_raw(*v)),
        S::State(v) => kebab(EventState::from_raw(*v)),
        S::SystemStatus(v) => kebab(DeviceStatus::from_raw(*v)),
        S::Units(v) => kebab(EngineeringUnits::from_raw(*v)),
        S::LifeSafetyMode(v) => kebab(LifeSafetyMode::from_raw(*v)),
        S::LifeSafetyState(v) => kebab(LifeSafetyState::from_raw(*v)),
        S::RestartReason(v) => kebab(RestartReason::from_raw(*v)),
        S::DoorAlarmState(v) => kebab(DoorAlarmState::from_raw(*v)),
        S::DoorSecuredStatus(v) => kebab(DoorSecuredStatus::from_raw(*v)),
        S::DoorStatus(v) => kebab(DoorStatus::from_raw(*v)),
        S::FileAccessMethod(v) => kebab(FileAccessMethod::from_raw(*v)),
        S::LockStatus(v) => kebab(LockStatus::from_raw(*v)),
        S::LifeSafetyOperation(v) => kebab(LifeSafetyOperation::from_raw(*v)),
        S::Maintenance(v) => kebab(Maintenance::from_raw(*v)),
        S::NodeType(v) => kebab(NodeType::from_raw(*v)),
        S::NotifyType(v) => kebab(NotifyType::from_raw(*v)),
        S::SilencedState(v) => kebab(SilencedState::from_raw(*v)),
        S::AccessEvent(v) => kebab(AccessEvent::from_raw(*v)),
        S::ZoneOccupancyState(v) => kebab(AccessZoneOccupancyState::from_raw(*v)),
        other => match other.as_u32() {
            Some(value) => format!("{} {value}", choice_name(other)),
            None => choice_name(other),
        },
    }
}

/// The name of a property state's choice, `shed-state` for `ShedState(3)`.
fn choice_name(state: &BACnetPropertyStates) -> String {
    let debug = format!("{state:?}");
    let name = debug.split('(').next().unwrap_or_default();
    let mut text = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            text.push('-');
        }
        text.push(ch.to_ascii_lowercase());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_written_as_runs() {
        let days = |bits| days_text(DaysOfWeek::from_bits_truncate(bits));
        assert_eq!(days(0b001_1111), "Mon-Fri");
        assert_eq!(days(0b111_1111), "every day");
        assert_eq!(days(0), "no days");
        assert_eq!(days(0b110_0101), "Mon, Wed, Sat-Sun");
        assert_eq!(days(0b000_0010), "Tue");
    }

    #[test]
    fn property_states_without_a_named_enumeration_keep_their_choice() {
        assert_eq!(
            property_state_text(&BACnetPropertyStates::ShedState(3)),
            "shed-state 3"
        );
        assert_eq!(
            property_state_text(&BACnetPropertyStates::State(3)),
            "high-limit"
        );
    }
}
