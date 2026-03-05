//! Device management services per ASHRAE 135-2020 Clauses 15-16.
//!
//! - DeviceCommunicationControl (Clause 15.4)
//! - ReinitializeDevice (Clause 15.4)
//! - TimeSynchronization (Clause 16.10)
//! - UTCTimeSynchronization (Clause 16.10)

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

// ---------------------------------------------------------------------------
// DeviceCommunicationControlRequest (Clause 15.4.1)
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
            time_duration = Some(primitives::decode_unsigned(content)? as u16);
            offset = new_offset;
        }

        // [1] enable-disable
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "DCC truncated at enable-disable"));
        }
        let enable_disable =
            EnableDisable::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [2] password (optional)
        let mut password = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 2)?;
            if let Some(content) = opt_data {
                let s = primitives::decode_character_string(content)?;
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
// ReinitializeDeviceRequest (Clause 15.4.2)
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
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "Reinitialize truncated at state"));
        }
        let reinitialized_state =
            ReinitializedState::from_raw(primitives::decode_unsigned(&data[pos..end])? as u32);
        offset = end;

        // [1] password (optional)
        let mut password = None;
        if offset < data.len() {
            let (opt_data, _new_offset) = tags::decode_optional_context(data, offset, 1)?;
            if let Some(content) = opt_data {
                let s = primitives::decode_character_string(content)?;
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
// TimeSynchronizationRequest (Clause 16.10.5)
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

        // App date (tag 10)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TimeSync truncated at date"));
        }
        let date = Date::decode(&data[pos..end])?;
        offset = end;

        // App time (tag 11)
        let (tag, pos) = tags::decode_tag(data, offset)?;
        let end = pos + tag.length as usize;
        if end > data.len() {
            return Err(Error::decoding(pos, "TimeSync truncated at time"));
        }
        let time = Time::decode(&data[pos..end])?;

        Ok(Self { date, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
