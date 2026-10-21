//! Device management services per ASHRAE 135-2020 Clause 16.
//!
//! - DeviceCommunicationControl (Clause 16.1)
//! - ReinitializeDevice (Clause 16.4)
//! - TimeSynchronization (§16.7)
//! - UTCTimeSynchronization (§16.8)

#[cfg(not(feature = "std"))]
use alloc::string::String;

use bacnet_encoding::constructed::tagged::{
    decode_app_fixed, decode_ctx_character_string, decode_ctx_unsigned, decode_optional_ctx,
    expect_end,
};
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
    /// Minutes the requested state lasts; `None` means until changed by a later request.
    pub time_duration: Option<u16>,
    /// Requested communication state for the device.
    pub enable_disable: EnableDisable,
    /// Password the device may require before honoring the request; `None` when not supplied.
    pub password: Option<String>,
}

impl DeviceCommunicationControlRequest {
    /// Append the ASN.1 encoding of the request to `buf`; fails if the password is unencodable.
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

    /// Decode the request from `data`; errors on malformed input, octets after the members, an
    /// out-of-range duration or a password longer than 20 characters.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] time-duration (optional)
        let (time_duration, offset) =
            decode_optional_ctx(data, 0, 0, "DCC time-duration", decode_ctx_unsigned::<u16>)?;

        // [1] enable-disable
        let (enable_disable, offset) =
            decode_ctx_unsigned::<u32>(data, offset, 1, "DCC enable-disable")?;
        let enable_disable = EnableDisable::from_raw(enable_disable);

        // [2] password (optional, max 20 characters), then nothing
        let (password, end) =
            decode_optional_ctx(data, offset, 2, "DCC password", decode_ctx_character_string)?;
        expect_end(data, end, end, "DCC")?;
        if password.as_ref().is_some_and(|s| s.len() > 20) {
            return Err(Error::encode("DCC password exceeds 20 characters"));
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
    /// Restart or backup/restore action the device is asked to perform.
    pub reinitialized_state: ReinitializedState,
    /// Password the device may require before honoring the request; `None` when not supplied.
    pub password: Option<String>,
}

impl ReinitializeDeviceRequest {
    /// Append the ASN.1 encoding of the request to `buf`; fails if the password is unencodable.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        // [0] reinitialized-state
        primitives::encode_ctx_enumerated(buf, 0, self.reinitialized_state.to_raw());
        // [1] password (optional)
        if let Some(ref pw) = self.password {
            primitives::encode_ctx_character_string(buf, 1, pw)?;
        }
        Ok(())
    }

    /// Decode the request from `data`; errors on missing or malformed fields and on octets after
    /// them.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        // [0] reinitialized-state
        let (state, offset) = decode_ctx_unsigned::<u32>(data, 0, 0, "Reinitialize state")?;
        let reinitialized_state = ReinitializedState::from_raw(state);

        // [1] password (optional, max 20 characters), then nothing
        let (password, end) = decode_optional_ctx(
            data,
            offset,
            1,
            "ReinitializeDevice password",
            decode_ctx_character_string,
        )?;
        expect_end(data, end, end, "ReinitializeDevice")?;
        if password.as_ref().is_some_and(|s| s.len() > 20) {
            return Err(Error::decoding(
                offset,
                "ReinitializeDevice password exceeds 20 characters",
            ));
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
    /// Calendar date to synchronize to.
    pub date: Date,
    /// Time of day to synchronize to.
    pub time: Time,
}

impl TimeSynchronizationRequest {
    /// Append the application-tagged encoding of the date and time to `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_app_date(buf, &self.date);
        primitives::encode_app_time(buf, &self.time);
    }

    /// Decode the request from `data`; errors on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let (date, offset) = decode_app_fixed(data, 0, tags::app_tag::DATE, 4, "TimeSync date")?;
        let date = Date::decode(date)?;
        let (time, end) = decode_app_fixed(data, offset, tags::app_tag::TIME, 4, "TimeSync time")?;
        let time = Time::decode(time)?;
        expect_end(data, end, end, "TimeSync")?;

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
        // The shared readers' wording names the member, its tag and the value.
        for (duration, enable_disable, field, value) in [
            (Some(65_536), 0, "time-duration: [0]", 65_536_u64),
            (None, 4_294_967_296, "enable-disable: [1]", 4_294_967_296),
            (Some(u64::MAX), 0, "time-duration: [0]", u64::MAX),
            (None, u64::MAX, "enable-disable: [1]", u64::MAX),
        ] {
            let encoded = encode_dcc(duration, enable_disable);
            let error = DeviceCommunicationControlRequest::decode(&encoded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("DCC {field} value {value} exceeds")),
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
                error.to_string().contains(&format!(
                    "Reinitialize state: [0] value {value} exceeds u32"
                )),
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
