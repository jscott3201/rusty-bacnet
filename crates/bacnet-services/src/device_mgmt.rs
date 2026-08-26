//! Device management services per ASHRAE 135-2020 Clauses 15-16.
//!
//! - DeviceCommunicationControl (Clause 15.4)
//! - ReinitializeDevice (Clause 15.4)
//! - TimeSynchronization (§16.7)
//! - UTCTimeSynchronization (§16.8)

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// DeviceCommunicationControlRequest
// ---------------------------------------------------------------------------

/// DeviceCommunicationControl-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCommunicationControlRequest {
    pub time_duration: Option<u16>,
    pub enable_disable: EnableDisable,
    pub password: Option<String>,
}

impl DeviceCommunicationControlRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] time-duration (optional)
        if let Some(dur) = self.time_duration {
            primitives::encode_ctx_unsigned(buf, 0, dur as u64);
        }
        // [1] enable-disable
        primitives::encode_ctx_enumerated(buf, 1, self.enable_disable.to_raw());
        // [2] password (optional)
        if let Some(ref pw) = self.password {
            primitives::encode_ctx_character_string(buf, 2, pw)?;
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] time-duration (optional)
        let mut time_duration = None;
        let (opt_data, new_offset) = tags::decode_optional_context(data, offset, 0)?;
        if let Some(content) = opt_data {
            let time_duration_raw = primitives::decode_unsigned(content)?;
            time_duration = Some(u16::try_from(time_duration_raw).map_err(|_| {
                Error::decoding(
                    offset,
                    format!("DCC time-duration {time_duration_raw} exceeds u16"),
                )
            })?);
            offset = new_offset;
        }

        // [1] enable-disable
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(1) {
            return Err(Error::decoding(offset, "DCC expected context tag 1"));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "DCC truncated at enable-disable"));
        }
        let enable_disable_raw = primitives::decode_unsigned(&data[pos..end])?;
        let enable_disable =
            EnableDisable::from_raw(u32::try_from(enable_disable_raw).map_err(|_| {
                Error::decoding(
                    pos,
                    format!("DCC enable-disable {enable_disable_raw} exceeds u32"),
                )
            })?);
        offset = end;

        // [2] password (optional, max 20 characters)
        let mut password = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 2)?;
            if let Some(content) = opt_data {
                let s = primitives::decode_character_string(content)?;
                if s.len() > 20 {
                    return Err(Error::Encoding("DCC password exceeds 20 characters".into()));
                }
                password = Some(s);
            }
        }

        Ok(Self {
            time_duration,
            enable_disable,
            password,
        })
    }
}

// ---------------------------------------------------------------------------
// ReinitializeDeviceRequest
// ---------------------------------------------------------------------------

/// ReinitializeDevice-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReinitializeDeviceRequest {
    pub reinitialized_state: ReinitializedState,
    pub password: Option<String>,
}

impl ReinitializeDeviceRequest {
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] reinitialized-state
        primitives::encode_ctx_enumerated(buf, 0, self.reinitialized_state.to_raw());
        // [1] password (optional)
        if let Some(ref pw) = self.password {
            primitives::encode_ctx_character_string(buf, 1, pw)?;
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        // [0] reinitialized-state
        let (tag, pos) = tags::decode_tag(data, offset)?;
        if !tag.is_context(0) {
            return Err(Error::decoding(
                offset,
                "Reinitialize expected context tag 0",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "Reinitialize truncated at state"));
        }
        let reinitialized_state_raw = primitives::decode_unsigned(&data[pos..end])?;
        let reinitialized_state =
            ReinitializedState::from_raw(u32::try_from(reinitialized_state_raw).map_err(|_| {
                Error::decoding(
                    pos,
                    format!("Reinitialize state {reinitialized_state_raw} exceeds u32"),
                )
            })?);
        offset = end;

        // [1] password (optional)
        let mut password = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 1)?;
            if let Some(content) = opt_data {
                let s = primitives::decode_character_string(content)?;
                if s.len() > 20 {
                    return Err(Error::decoding(
                        offset,
                        "ReinitializeDevice password exceeds 20 characters",
                    ));
                }
                password = Some(s);
            }
        }

        Ok(Self {
            reinitialized_state,
            password,
        })
    }
}

// ---------------------------------------------------------------------------
// TimeSynchronizationRequest
// ---------------------------------------------------------------------------

/// TimeSynchronization-Request service parameters (APPLICATION-tagged).
///
/// Used for both TimeSynchronization and UTCTimeSynchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSynchronizationRequest {
    pub date: Date,
    pub time: Time,
}

impl TimeSynchronizationRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_date(buf, &self.date);
        primitives::encode_app_time(buf, &self.time);
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::DATE
            || tag.length != 4
        {
            return Err(Error::decoding(
                offset,
                "TimeSync expected application Date",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TimeSync truncated at date"));
        }
        let date = Date::decode(&data[pos..end])?;
        offset = end;

        let (tag, pos) = tags::decode_tag(data, offset)?;
        if tag.class != tags::TagClass::Application
            || tag.number != tags::app_tag::TIME
            || tag.length != 4
        {
            return Err(Error::decoding(
                offset,
                "TimeSync expected application Time",
            ));
        }
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TimeSync truncated at time"));
        }
        let time = Time::decode(&data[pos..end])?;
        if end != data.len() {
            return Err(Error::decoding(end, "TimeSync contains trailing data"));
        }

        Ok(Self { date, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_dcc(time_duration: Option<u64>, enable_disable: u64) -> BytesMut {
        let mut buf = BytesMut::new();
        if let Some(duration) = time_duration {
            primitives::encode_ctx_unsigned(&mut buf, 0, duration);
        }
        primitives::encode_ctx_unsigned(&mut buf, 1, enable_disable);
        buf
    }

    fn encode_reinitialize(state: u64) -> BytesMut {
        let mut buf = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut buf, 0, state);
        buf
    }

    #[test]
    fn dcc_round_trip() {
        let req = DeviceCommunicationControlRequest {
            time_duration: Some(60),
            enable_disable: EnableDisable::DISABLE,
            password: Some("secret".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = DeviceCommunicationControlRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn dcc_no_optionals() {
        let req = DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::ENABLE,
            password: None,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = DeviceCommunicationControlRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn dcc_values_must_fit_field_widths() {
        for (duration, enable_disable, field, value) in [
            (Some(65_536), 0, "time-duration", 65_536_u64),
            (None, 4_294_967_296, "enable-disable", 4_294_967_296),
            (Some(u64::MAX), 0, "time-duration", u64::MAX),
            (None, u64::MAX, "enable-disable", u64::MAX),
        ] {
            let encoded = encode_dcc(duration, enable_disable);
            let error = DeviceCommunicationControlRequest::decode(&encoded).unwrap_err();
            assert!(
                error.to_string().contains(&format!("DCC {field} {value}")),
                "unexpected error for {field} {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(&mut leading_zero, 0, tags::TagClass::Context, 3);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff]);
        tags::encode_tag(&mut leading_zero, 1, tags::TagClass::Context, 5);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        let decoded = DeviceCommunicationControlRequest::decode(&leading_zero).unwrap();
        assert_eq!(decoded.time_duration, Some(u16::MAX));
        assert_eq!(decoded.enable_disable.to_raw(), u32::MAX);
    }

    #[test]
    fn dcc_requires_enable_disable_context_tag() {
        let mut application_tag = BytesMut::new();
        primitives::encode_app_enumerated(&mut application_tag, 0);
        assert!(DeviceCommunicationControlRequest::decode(&application_tag).is_err());

        let mut wrong_context_tag = BytesMut::new();
        primitives::encode_ctx_enumerated(&mut wrong_context_tag, 2, 0);
        assert!(DeviceCommunicationControlRequest::decode(&wrong_context_tag).is_err());
    }

    #[test]
    fn reinitialize_round_trip() {
        let req = ReinitializeDeviceRequest {
            reinitialized_state: ReinitializedState::WARMSTART,
            password: Some("admin".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = ReinitializeDeviceRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn reinitialize_state_must_fit_u32() {
        for value in [4_294_967_296, 4_294_967_297, u64::MAX] {
            let encoded = encode_reinitialize(value);
            let error = ReinitializeDeviceRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("Reinitialize state {value}")),
                "unexpected error for state {value}: {error}"
            );
        }

        let mut leading_zero = BytesMut::new();
        tags::encode_tag(&mut leading_zero, 0, tags::TagClass::Context, 5);
        leading_zero.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff]);
        let decoded = ReinitializeDeviceRequest::decode(&leading_zero).unwrap();
        assert_eq!(decoded.reinitialized_state.to_raw(), u32::MAX);
    }

    #[test]
    fn reinitialize_requires_state_context_tag() {
        let mut application_tag = BytesMut::new();
        primitives::encode_app_enumerated(&mut application_tag, 0);
        assert!(ReinitializeDeviceRequest::decode(&application_tag).is_err());

        let mut wrong_context_tag = BytesMut::new();
        primitives::encode_ctx_enumerated(&mut wrong_context_tag, 1, 0);
        assert!(ReinitializeDeviceRequest::decode(&wrong_context_tag).is_err());
    }

    #[test]
    fn time_sync_round_trip() {
        let req = TimeSynchronizationRequest {
            date: Date {
                year: 124,
                month: 6,
                day: 15,
                day_of_week: 6,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let decoded = TimeSynchronizationRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn time_sync_requires_exact_application_tags_and_no_trailing_data() {
        let mut wrong_date = BytesMut::new();
        primitives::encode_app_time(
            &mut wrong_date,
            &Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            },
        );
        primitives::encode_app_time(
            &mut wrong_date,
            &Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            },
        );
        assert!(TimeSynchronizationRequest::decode(&wrong_date).is_err());

        let request = TimeSynchronizationRequest {
            date: Date {
                year: 124,
                month: 7,
                day: 4,
                day_of_week: 4,
            },
            time: Time {
                hour: 9,
                minute: 15,
                second: 0,
                hundredths: 0,
            },
        };
        let mut trailing = BytesMut::new();
        request.encode(&mut trailing);
        trailing.extend_from_slice(&[0]);
        assert!(TimeSynchronizationRequest::decode(&trailing).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_dcc_empty_input() {
        assert!(DeviceCommunicationControlRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_dcc_truncated_1_byte() {
        let req = DeviceCommunicationControlRequest {
            time_duration: Some(60),
            enable_disable: EnableDisable::DISABLE,
            password: Some("secret".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(DeviceCommunicationControlRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_dcc_truncated_3_bytes() {
        let req = DeviceCommunicationControlRequest {
            time_duration: Some(60),
            enable_disable: EnableDisable::DISABLE,
            password: Some("secret".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(DeviceCommunicationControlRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_dcc_invalid_tag() {
        assert!(DeviceCommunicationControlRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_reinitialize_empty_input() {
        assert!(ReinitializeDeviceRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_reinitialize_truncated_1_byte() {
        let req = ReinitializeDeviceRequest {
            reinitialized_state: ReinitializedState::WARMSTART,
            password: Some("admin".into()),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(ReinitializeDeviceRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_reinitialize_invalid_tag() {
        assert!(ReinitializeDeviceRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn test_decode_time_sync_empty_input() {
        assert!(TimeSynchronizationRequest::decode(&[]).is_err());
    }

    #[test]
    fn test_decode_time_sync_truncated_1_byte() {
        let req = TimeSynchronizationRequest {
            date: Date {
                year: 124,
                month: 6,
                day: 15,
                day_of_week: 6,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(TimeSynchronizationRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_time_sync_truncated_3_bytes() {
        let req = TimeSynchronizationRequest {
            date: Date {
                year: 124,
                month: 6,
                day: 15,
                day_of_week: 6,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert!(TimeSynchronizationRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_time_sync_truncated_half() {
        let req = TimeSynchronizationRequest {
            date: Date {
                year: 124,
                month: 6,
                day: 15,
                day_of_week: 6,
            },
            time: Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let half = buf.len() / 2;
        assert!(TimeSynchronizationRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_time_sync_invalid_tag() {
        assert!(TimeSynchronizationRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
