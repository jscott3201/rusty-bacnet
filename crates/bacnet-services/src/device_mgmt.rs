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
use bacnet_types::error::{DecodingKind, Error};
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;

/// Longest password, in characters, the DCC and ReinitializeDevice requests
/// carry (Clauses 16.1.1.1.3 and 16.4.1.1.2).
pub const MAX_PASSWORD_CHARS: usize = 20;

/// Whether `password` is within [`MAX_PASSWORD_CHARS`]. The limit counts
/// characters, not octets, so a multi-octet UTF-8 character counts once. A
/// server or endpoint checks its configured password with it, since a longer
/// one could never be sent.
///
/// ```
/// use bacnet_services::device_mgmt::password_fits;
///
/// assert!(password_fits(&"é".repeat(20)));
/// assert!(!password_fits(&"a".repeat(21)));
/// ```
pub fn password_fits(password: &str) -> bool {
    password.chars().count() <= MAX_PASSWORD_CHARS
}

/// Whether a decoded password is longer than [`MAX_PASSWORD_CHARS`]. The
/// decoder has already converted UTF-8, UCS-2 and ISO 8859-1 contents to a
/// `String`, one `char` per character of the sender's set.
fn password_too_long(password: Option<&String>) -> bool {
    password.is_some_and(|s| !password_fits(s))
}

/// A decoded password longer than [`MAX_PASSWORD_CHARS`]: its encoding is
/// valid, only its size is outside the service's range, so a responder
/// rejects it as PARAMETER_OUT_OF_RANGE (#2085). `offset` is where the
/// password member starts.
fn password_out_of_range(offset: usize, label: &'static str) -> Error {
    Error::decoding_kind(DecodingKind::OutOfRange, offset, label)
}

/// Whether `password` can be configured or sent: at least one character
/// and at most [`MAX_PASSWORD_CHARS`] (#2094). An empty password is out of
/// the service's range; a caller omits it (`None`) instead.
///
/// ```
/// use bacnet_services::device_mgmt::password_valid;
///
/// assert!(password_valid("secret"));
/// assert!(!password_valid(""));
/// assert!(!password_valid(&"a".repeat(21)));
/// ```
pub fn password_valid(password: &str) -> bool {
    !password.is_empty() && password_fits(password)
}

/// Refuse to encode a password the peer would only refuse, so the caller
/// hears about it before anything is sent: one longer than
/// [`MAX_PASSWORD_CHARS`] (#2085), or an empty one, which the caller should
/// omit instead (#2094). `too_long` and `empty` name the request.
fn check_password(
    password: Option<&str>,
    too_long: &'static str,
    empty: &'static str,
) -> Result<(), Error> {
    match password {
        Some("") => Err(Error::encode(empty)),
        Some(pw) if !password_fits(pw) => Err(Error::encode(too_long)),
        _ => Ok(()),
    }
}

/// A decoded password, with an empty one taken as absent (#2094): some
/// devices send an empty string for "no password", and a server then treats
/// the request as one without a password.
fn present(password: Option<String>) -> Option<String> {
    password.filter(|pw| !pw.is_empty())
}

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
    /// Password the device may require before honoring the request, 1 to 20 characters; `None`
    /// when not supplied.
    pub password: Option<String>,
}

impl DeviceCommunicationControlRequest {
    /// Append the ASN.1 encoding of the request to `buf`. Fails, leaving `buf` unchanged, if the
    /// password is empty (omit it instead) or longer than [`MAX_PASSWORD_CHARS`] characters;
    /// fails if it is unencodable.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        check_password(
            self.password.as_deref(),
            "DCC password exceeds the 20-character limit",
            "DCC password is empty; omit it (None) to send no password",
        )?;
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
    /// out-of-range duration or a password longer than 20 characters. An empty password decodes
    /// as none.
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
        if password_too_long(password.as_ref()) {
            return Err(password_out_of_range(
                offset,
                "DCC password exceeds 20 characters",
            ));
        }

        Ok(Self {
            time_duration,
            enable_disable,
            password: present(password),
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
    /// Password the device may require before honoring the request, 1 to 20 characters; `None`
    /// when not supplied.
    pub password: Option<String>,
}

impl ReinitializeDeviceRequest {
    /// Append the ASN.1 encoding of the request to `buf`. Fails, leaving `buf` unchanged, if the
    /// password is empty (omit it instead) or longer than [`MAX_PASSWORD_CHARS`] characters;
    /// fails if it is unencodable.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        check_password(
            self.password.as_deref(),
            "ReinitializeDevice password exceeds the 20-character limit",
            "ReinitializeDevice password is empty; omit it (None) to send no password",
        )?;
        // [0] reinitialized-state
        primitives::encode_ctx_enumerated(buf, 0, self.reinitialized_state.to_raw());
        // [1] password (optional)
        if let Some(ref pw) = self.password {
            primitives::encode_ctx_character_string(buf, 1, pw)?;
        }
        Ok(())
    }

    /// Decode the request from `data`; errors on missing or malformed fields, on octets after
    /// them and on a password longer than 20 characters. An empty password decodes as none.
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
        if password_too_long(password.as_ref()) {
            return Err(password_out_of_range(
                offset,
                "ReinitializeDevice password exceeds 20 characters",
            ));
        }

        Ok(Self {
            reinitialized_state,
            password: present(password),
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

    /// Encode a DCC (enable) and a ReinitializeDevice (coldstart) request whose
    /// password member holds `charset` then `payload`, uninterpreted.
    fn raw_password_requests(charset: u8, payload: &[u8]) -> [BytesMut; 2] {
        let len = u32::try_from(payload.len()).unwrap() + 1;
        let mut dcc = encode_dcc(None, 0);
        tags::encode_tag(&mut dcc, 2, tags::TagClass::Context, len);
        let mut reinit = encode_reinitialize(0);
        tags::encode_tag(&mut reinit, 1, tags::TagClass::Context, len);
        for buf in [&mut dcc, &mut reinit] {
            buf.extend_from_slice(&[charset]);
            buf.extend_from_slice(payload);
        }
        [dcc, reinit]
    }

    /// Decode both requests and return their passwords, or `None` for each
    /// one refused.
    fn decoded_passwords([dcc, reinit]: [BytesMut; 2]) -> [Option<String>; 2] {
        [
            DeviceCommunicationControlRequest::decode(&dcc)
                .ok()
                .and_then(|r| r.password),
            ReinitializeDeviceRequest::decode(&reinit)
                .ok()
                .and_then(|r| r.password),
        ]
    }

    #[test]
    fn password_limit_counts_characters_not_octets() {
        // 20 characters each, 40 to 80 octets in UTF-8 (#2060).
        for ch in ["é", "パ", "😀", "a"] {
            let twenty = ch.repeat(20);
            let dcc = DeviceCommunicationControlRequest {
                time_duration: None,
                enable_disable: EnableDisable::ENABLE,
                password: Some(twenty.clone()),
            };
            let reinit = ReinitializeDeviceRequest {
                reinitialized_state: ReinitializedState::WARMSTART,
                password: Some(twenty),
            };
            let mut buf = BytesMut::new();
            dcc.encode(&mut buf).unwrap();
            assert_eq!(DeviceCommunicationControlRequest::decode(&buf).unwrap(), dcc);
            buf.clear();
            reinit.encode(&mut buf).unwrap();
            assert_eq!(ReinitializeDeviceRequest::decode(&buf).unwrap(), reinit);

            // The encoders refuse 21 characters (#2085), so the decoder
            // sees them as raw UTF-8 (character set 0).
            let twenty_one = ch.repeat(21);
            assert_eq!(
                decoded_passwords(raw_password_requests(0, twenty_one.as_bytes())),
                [None, None]
            );
        }
    }

    #[test]
    fn encoders_refuse_a_password_over_the_limit() {
        for ch in ["é", "パ", "😀", "a"] {
            let dcc = DeviceCommunicationControlRequest {
                time_duration: Some(5),
                enable_disable: EnableDisable::DISABLE,
                password: Some(ch.repeat(21)),
            };
            let reinit = ReinitializeDeviceRequest {
                reinitialized_state: ReinitializedState::COLDSTART,
                password: Some(ch.repeat(21)),
            };
            let mut buf = BytesMut::new();
            let err = dcc.encode(&mut buf).unwrap_err();
            assert!(matches!(err, Error::Encode(_)), "{err:?}");
            assert!(err.to_string().contains("20-character limit"), "{err}");
            assert!(buf.is_empty());
            let err = reinit.encode(&mut buf).unwrap_err();
            assert!(matches!(err, Error::Encode(_)), "{err:?}");
            assert!(err.to_string().contains("20-character limit"), "{err}");
            assert!(buf.is_empty());
        }
    }

    #[test]
    fn password_limit_counts_characters_of_the_sent_character_set() {
        // ISO 8859-1: one octet a character, even above 0x7F (two UTF-8 octets).
        let latin1 = [0xe9; 21];
        let expected = "é".repeat(20);
        assert_eq!(
            decoded_passwords(raw_password_requests(5, &latin1[..20])),
            [Some(expected.clone()), Some(expected)]
        );
        assert_eq!(
            decoded_passwords(raw_password_requests(5, &latin1)),
            [None, None]
        );
        // UCS-2: two octets a character (U+30D1, three UTF-8 octets).
        let ucs2 = [0x30, 0xd1].repeat(21);
        let expected = "\u{30d1}".repeat(20);
        assert_eq!(
            decoded_passwords(raw_password_requests(4, &ucs2[..40])),
            [Some(expected.clone()), Some(expected)]
        );
        assert_eq!(decoded_passwords(raw_password_requests(4, &ucs2)), [None, None]);
    }

    /// #2094: the encoders refuse an empty password, telling the caller to
    /// omit it, and leave the buffer untouched.
    #[test]
    fn encoders_refuse_an_empty_password() {
        let dcc = DeviceCommunicationControlRequest {
            time_duration: None,
            enable_disable: EnableDisable::ENABLE,
            password: Some(String::new()),
        };
        let reinit = ReinitializeDeviceRequest {
            reinitialized_state: ReinitializedState::WARMSTART,
            password: Some(String::new()),
        };
        let mut buf = BytesMut::new();
        for err in [
            dcc.encode(&mut buf).unwrap_err(),
            reinit.encode(&mut buf).unwrap_err(),
        ] {
            assert!(matches!(err, Error::Encode(_)), "{err:?}");
            assert!(err.to_string().contains("omit it"), "{err}");
        }
        assert!(buf.is_empty());
    }

    /// #2094: an empty password from a peer decodes as no password, in every
    /// character set, so a server checks the request as one without it.
    #[test]
    fn decoders_take_an_empty_password_as_absent() {
        for charset in [0, 4, 5] {
            let [dcc, reinit] = raw_password_requests(charset, &[]);
            let dcc = DeviceCommunicationControlRequest::decode(&dcc).unwrap();
            assert_eq!(dcc.password, None);
            assert_eq!(dcc.enable_disable, EnableDisable::ENABLE);
            let reinit = ReinitializeDeviceRequest::decode(&reinit).unwrap();
            assert_eq!(reinit.password, None);
            assert_eq!(reinit.reinitialized_state, ReinitializedState::COLDSTART);
        }
        // One character is still a password.
        assert_eq!(
            decoded_passwords(raw_password_requests(0, b"x")),
            [Some("x".into()), Some("x".into())]
        );
    }
}
