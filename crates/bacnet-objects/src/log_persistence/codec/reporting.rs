//! The written reporting settings at the end of a schema version 2 slot
//! (#1952).
//!
//! A big-endian 16-bit presence mask comes first, one bit per setting in the
//! order of [`LogReportingSettings`]' fields, then each setting present, in
//! that order:
//!
//! - Notification_Class: four octets;
//! - Event_Enable: one octet holding the transition bits;
//! - Notify_Type: its four-octet enumeration value;
//! - Event_Detection_Enable: one Boolean octet;
//! - Event_Message_Texts_Config: three length-prefixed UTF-8 strings;
//! - Event_Algorithm_Inhibit_Ref: a length-prefixed Clause 21 encoding of the
//!   reference;
//! - Event_Algorithm_Inhibit: one Boolean octet;
//! - Client_COV_Increment: a choice octet, 0 for NULL, or 1 followed by the
//!   four octets of the REAL;
//! - COV_Resubscription_Interval: four octets.
//!
//! A mask bit this schema doesn't name is damage, as is any value a write
//! of that property would refuse.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_object_property_reference, encode_object_property_reference,
};
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{NotifyType, ObjectType};
use bytes::BytesMut;

use super::{MAX_ELEMENT_BYTES, Reader, corrupt};
use crate::log_persistence::LogReportingSettings;
use crate::snapshot::SnapshotError;

/// The most octets one Event_Message_Texts_Config string may take in a
/// slot, so the three fit the room [`super::FIXED_ALLOWANCE`] keeps beside
/// the records. A durable log refuses a longer one as it is written, with
/// RESOURCES / NO_SPACE_TO_WRITE_PROPERTY, whatever its storage (#1952).
pub(crate) const MAX_TEXT_OCTETS: usize = crate::event::options::MAX_KEPT_TEXT_OCTETS;

const CLASS: u16 = 1 << 0;
const ENABLE: u16 = 1 << 1;
const NOTIFY_TYPE: u16 = 1 << 2;
const DETECTION: u16 = 1 << 3;
const TEXTS: u16 = 1 << 4;
const INHIBIT_REF: u16 = 1 << 5;
const INHIBIT: u16 = 1 << 6;
const INCREMENT: u16 = 1 << 7;
const RESUBSCRIPTION: u16 = 1 << 8;
const KNOWN: u16 = (1 << 9) - 1;

/// Refuse settings no write could leave on an `kind` log: COV settings off
/// a Trend Log, an Event_Enable or Notify_Type outside its enumeration, an
/// increment that is negative or not finite, a zero resubscription
/// interval, a message text past [`MAX_TEXT_OCTETS`], or an inhibit
/// reference in the unset form, which is kept as none.
pub(super) fn validate(
    kind: ObjectType,
    settings: &LogReportingSettings,
) -> Result<(), SnapshotError> {
    let cov =
        settings.client_cov_increment.is_some() || settings.cov_resubscription_interval.is_some();
    if cov && kind != ObjectType::TREND_LOG {
        return Err(corrupt("holds COV settings only a Trend Log has"));
    }
    if let Some(enable) = settings.event_enable
        && EventTransitionBits::from_bits(enable.bits()).is_none()
    {
        return Err(corrupt("holds unknown Event_Enable bits"));
    }
    if let Some(notify_type) = settings.notify_type
        && !NotifyType::ALL_NAMED
            .iter()
            .any(|&(_, named)| named == notify_type)
    {
        return Err(corrupt("holds an unknown Notify_Type"));
    }
    if let Some(Some(increment)) = settings.client_cov_increment
        && (!increment.is_finite() || increment < 0.0)
    {
        return Err(corrupt("holds a Client_COV_Increment no write takes"));
    }
    if settings.cov_resubscription_interval == Some(0) {
        return Err(corrupt("holds a zero COV_Resubscription_Interval"));
    }
    if let Some(texts) = &settings.event_message_texts_config
        && texts.iter().any(|text| text.len() > MAX_TEXT_OCTETS)
    {
        return Err(SnapshotError::out_of_range(
            "holds an Event_Message_Texts_Config string past the 16 KiB a log keeps",
        ));
    }
    if settings
        .event_algorithm_inhibit_ref
        .as_ref()
        .is_some_and(|reference| reference.is_unset())
    {
        return Err(corrupt("holds an unset Event_Algorithm_Inhibit_Ref"));
    }
    Ok(())
}

/// Append `settings`, already [validated](validate), to `data`; `element`
/// is scratch space for the reference's encoding.
pub(super) fn put(data: &mut Vec<u8>, settings: &LogReportingSettings, element: &mut BytesMut) {
    let mask = [
        (settings.notification_class.is_some(), CLASS),
        (settings.event_enable.is_some(), ENABLE),
        (settings.notify_type.is_some(), NOTIFY_TYPE),
        (settings.event_detection_enable.is_some(), DETECTION),
        (settings.event_message_texts_config.is_some(), TEXTS),
        (settings.event_algorithm_inhibit_ref.is_some(), INHIBIT_REF),
        (settings.event_algorithm_inhibit.is_some(), INHIBIT),
        (settings.client_cov_increment.is_some(), INCREMENT),
        (
            settings.cov_resubscription_interval.is_some(),
            RESUBSCRIPTION,
        ),
    ]
    .into_iter()
    .filter(|(present, _)| *present)
    .fold(0, |mask, (_, bit)| mask | bit);
    data.extend_from_slice(&mask.to_be_bytes());
    if let Some(class) = settings.notification_class {
        data.extend_from_slice(&class.to_be_bytes());
    }
    if let Some(enable) = settings.event_enable {
        data.push(enable.bits());
    }
    if let Some(notify_type) = settings.notify_type {
        data.extend_from_slice(&notify_type.to_raw().to_be_bytes());
    }
    if let Some(enable) = settings.event_detection_enable {
        data.push(u8::from(enable));
    }
    if let Some(texts) = &settings.event_message_texts_config {
        for text in texts {
            put_octets(data, text.as_bytes());
        }
    }
    if let Some(reference) = &settings.event_algorithm_inhibit_ref {
        element.clear();
        encode_object_property_reference(element, reference);
        put_octets(data, element);
    }
    if let Some(inhibit) = settings.event_algorithm_inhibit {
        data.push(u8::from(inhibit));
    }
    match settings.client_cov_increment {
        None => {}
        Some(None) => data.push(0),
        Some(Some(increment)) => {
            data.push(1);
            data.extend_from_slice(&increment.to_bits().to_be_bytes());
        }
    }
    if let Some(seconds) = settings.cov_resubscription_interval {
        data.extend_from_slice(&seconds.to_be_bytes());
    }
}

/// A length, then that many octets. [`validate`] keeps every string and
/// reference here well under a 32-bit length.
fn put_octets(data: &mut Vec<u8>, octets: &[u8]) {
    data.extend_from_slice(&(octets.len() as u32).to_be_bytes());
    data.extend_from_slice(octets);
}

/// Read the settings [`put`] wrote; the caller validates the snapshot whole.
pub(super) fn read(reader: &mut Reader<'_>) -> Result<LogReportingSettings, SnapshotError> {
    let mask = u16::from_be_bytes(
        reader
            .take(2, "reporting presence")?
            .try_into()
            .expect("two octets"),
    );
    if mask & !KNOWN != 0 {
        return Err(corrupt("reporting presence holds unknown bits"));
    }
    let has = |bit: u16| mask & bit != 0;
    let mut settings = LogReportingSettings::default();
    if has(CLASS) {
        settings.notification_class = Some(reader.u32("Notification_Class")?);
    }
    if has(ENABLE) {
        let bits = reader.u8("Event_Enable")?;
        settings.event_enable = Some(
            EventTransitionBits::from_bits(bits)
                .ok_or_else(|| corrupt("holds unknown Event_Enable bits"))?,
        );
    }
    if has(NOTIFY_TYPE) {
        settings.notify_type = Some(NotifyType::from_raw(reader.u32("Notify_Type")?));
    }
    if has(DETECTION) {
        settings.event_detection_enable = Some(reader.flag("Event_Detection_Enable")?);
    }
    if has(TEXTS) {
        let mut text = || {
            let octets = octets(reader, "Event_Message_Texts_Config", MAX_TEXT_OCTETS)?;
            String::from_utf8(octets.to_vec())
                .map_err(|_| corrupt("Event_Message_Texts_Config is not UTF-8"))
        };
        settings.event_message_texts_config = Some([text()?, text()?, text()?]);
    }
    if has(INHIBIT_REF) {
        let octets = octets(reader, "Event_Algorithm_Inhibit_Ref", MAX_ELEMENT_BYTES)?;
        settings.event_algorithm_inhibit_ref = Some(
            decode_object_property_reference(octets)
                .map_err(|_| corrupt("does not decode").in_field("Event_Algorithm_Inhibit_Ref"))?,
        );
    }
    if has(INHIBIT) {
        settings.event_algorithm_inhibit = Some(reader.flag("Event_Algorithm_Inhibit")?);
    }
    if has(INCREMENT) {
        settings.client_cov_increment = Some(match reader.u8("Client_COV_Increment")? {
            0 => None,
            1 => Some(f32::from_bits(reader.u32("Client_COV_Increment")?)),
            _ => return Err(corrupt("Client_COV_Increment holds an unknown choice")),
        });
    }
    if has(RESUBSCRIPTION) {
        settings.cov_resubscription_interval = Some(reader.u32("COV_Resubscription_Interval")?);
    }
    Ok(settings)
}

/// A length of at most `cap`, then that many octets.
fn octets<'a>(
    reader: &mut Reader<'a>,
    field: &'static str,
    cap: usize,
) -> Result<&'a [u8], SnapshotError> {
    let length = reader.u32(field)? as usize;
    if length > cap {
        return Err(corrupt("is too long").in_field(field));
    }
    reader.take(length, field)
}
