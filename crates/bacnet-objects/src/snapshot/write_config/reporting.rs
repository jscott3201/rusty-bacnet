//! The body of a [`KIND_REPORTING`](super::KIND_REPORTING) file: the
//! reporting settings an intrinsic reporter keeps (#2331). The parent
//! module documents the layout.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_object_property_reference, encode_object_property_reference,
};
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{NotifyType, ObjectType};
use bytes::BytesMut;

use super::{Reader, SnapshotError, put_octets};
use crate::event::options::MAX_KEPT_TEXT_OCTETS;
use crate::write_config::ReportingWriteConfig;

/// The most octets an Event_Algorithm_Inhibit_Ref may take: its object
/// identifier, property and array index, each context tagged, fit in 15.
const MAX_REFERENCE_OCTETS: usize = 64;

/// Whether objects of `object_type` keep their reporting settings in a
/// [`KIND_REPORTING`](super::KIND_REPORTING) file: the analog, binary and
/// multi-state Input, Output and Value objects, Access Door, Access Zone,
/// Event Enrollment and Alert Enrollment. A file of that kind for any other
/// object type is refused.
pub fn keeps_reporting(object_type: ObjectType) -> bool {
    matches!(
        object_type,
        ObjectType::ACCESS_DOOR
            | ObjectType::ACCESS_ZONE
            | ObjectType::EVENT_ENROLLMENT
            | ObjectType::ALERT_ENROLLMENT
            | ObjectType::ANALOG_INPUT
            | ObjectType::ANALOG_OUTPUT
            | ObjectType::ANALOG_VALUE
            | ObjectType::BINARY_INPUT
            | ObjectType::BINARY_OUTPUT
            | ObjectType::BINARY_VALUE
            | ObjectType::MULTI_STATE_INPUT
            | ObjectType::MULTI_STATE_OUTPUT
            | ObjectType::MULTI_STATE_VALUE
    )
}

/// Why `config` holds what no write leaves: Event_Enable bits past the
/// three transitions, a Notify_Type outside its enumeration, a message text
/// past the cap, or an inhibit reference in the unset form, which is kept
/// as none.
pub(crate) fn invalid(config: &ReportingWriteConfig) -> Option<&'static str> {
    if config
        .event_enable
        .is_some_and(|enable| EventTransitionBits::from_bits(enable.bits()).is_none())
    {
        return Some("holds unknown Event_Enable bits");
    }
    if config.notify_type.is_some_and(|notify_type| {
        !NotifyType::ALL_NAMED
            .iter()
            .any(|&(_, named)| named == notify_type)
    }) {
        return Some("holds an unknown Notify_Type");
    }
    if config
        .event_message_texts_config
        .as_ref()
        .is_some_and(|texts| texts.iter().any(|text| text.len() > MAX_KEPT_TEXT_OCTETS))
    {
        return Some("holds an Event_Message_Texts_Config string past its cap");
    }
    if config
        .event_algorithm_inhibit_ref
        .as_ref()
        .is_some_and(|reference| reference.is_unset())
    {
        return Some("holds an unset Event_Algorithm_Inhibit_Ref");
    }
    None
}

/// Append `config`'s body, refusing one [`invalid`] names.
pub(super) fn encode(
    body: &mut BytesMut,
    config: &ReportingWriteConfig,
) -> Result<(), SnapshotError> {
    if let Some(reason) = invalid(config) {
        return Err(SnapshotError::out_of_range(reason));
    }
    put(body, config.notification_class, |body, class| {
        body.extend_from_slice(&class.to_be_bytes());
    });
    put(body, config.event_enable, |body, enable| {
        body.extend_from_slice(&[enable.bits()]);
    });
    put(body, config.notify_type, |body, notify_type| {
        body.extend_from_slice(&notify_type.to_raw().to_be_bytes());
    });
    put(body, config.event_detection_enable, |body, enable| {
        body.extend_from_slice(&[u8::from(enable)]);
    });
    put(
        body,
        config.event_message_texts_config.as_ref(),
        |body, texts| {
            for text in texts {
                put_octets(body, text.as_bytes());
            }
        },
    );
    put(
        body,
        config.event_algorithm_inhibit_ref.as_ref(),
        |body, reference| {
            let mut octets = BytesMut::new();
            encode_object_property_reference(&mut octets, reference);
            put_octets(body, &octets);
        },
    );
    put(body, config.event_algorithm_inhibit, |body, inhibit| {
        body.extend_from_slice(&[u8::from(inhibit)]);
    });
    Ok(())
}

/// A presence octet, then the part if there is one.
fn put<T>(body: &mut BytesMut, part: Option<T>, encode: impl FnOnce(&mut BytesMut, T)) {
    match part {
        None => body.extend_from_slice(&[0]),
        Some(part) => {
            body.extend_from_slice(&[1]);
            encode(body, part);
        }
    }
}

impl Reader<'_> {
    /// A part behind its presence octet.
    fn part<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, SnapshotError>,
    ) -> Result<Option<T>, SnapshotError> {
        if self.flag()? {
            read(self).map(Some)
        } else {
            Ok(None)
        }
    }

    /// A reporting body, refused as damage where [`invalid`] would refuse
    /// it.
    pub(super) fn reporting(&mut self) -> Result<ReportingWriteConfig, SnapshotError> {
        let notification_class = self.part(Self::u32)?;
        let event_enable = self.part(|reader| {
            EventTransitionBits::from_bits(reader.u8()?)
                .ok_or(SnapshotError::damaged("holds unknown Event_Enable bits"))
        })?;
        let notify_type = self.part(|reader| Ok(NotifyType::from_raw(reader.u32()?)))?;
        let event_detection_enable = self.part(Self::flag)?;
        let event_message_texts_config = self.part(|reader| {
            let mut text = || {
                let octets = reader.octets(MAX_KEPT_TEXT_OCTETS)?;
                String::from_utf8(octets.to_vec())
                    .map_err(|_| SnapshotError::damaged("holds a message text that isn't UTF-8"))
            };
            Ok([text()?, text()?, text()?])
        })?;
        let event_algorithm_inhibit_ref = self.part(|reader| {
            decode_object_property_reference(reader.octets(MAX_REFERENCE_OCTETS)?)
                .map_err(|_| SnapshotError::damaged("holds an inhibit reference that won't decode"))
        })?;
        let event_algorithm_inhibit = self.part(Self::flag)?;
        let config = ReportingWriteConfig {
            notification_class,
            event_enable,
            notify_type,
            event_detection_enable,
            event_message_texts_config,
            event_algorithm_inhibit_ref,
            event_algorithm_inhibit,
            description: None,
            algorithm: Default::default(),
        };
        match invalid(&config) {
            Some(reason) => Err(SnapshotError::damaged(reason)),
            None => Ok(config),
        }
    }
}
