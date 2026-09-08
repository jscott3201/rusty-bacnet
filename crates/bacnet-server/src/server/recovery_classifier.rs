//! Local admission classification only; never authorizes or mutates DCC state.
use super::Class;
use bacnet_encoding::tags;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::enums::{ConfirmedServiceChoice, EnableDisable};

pub(in crate::server) fn confirmed_class(service: ConfirmedServiceChoice, data: &[u8]) -> Class {
    if service == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
        && bounded_password(data)
        && DeviceCommunicationControlRequest::decode(data)
            .is_ok_and(|request| request.enable_disable == EnableDisable::ENABLE)
    {
        Class::Recovery
    } else {
        Class::Confirmed
    }
}

// Mirror only the decoder's borrowed field traversal, not its accepted language.
// Its final decode remains authoritative, including optional duration, charset
// validity and trailing-data tolerance. Every accepted password has <=20 UTF-8
// bytes: UTF-8/Latin-1 need <=20 wire payload bytes, UCS-2 <=40 (each code point
// produces at least one UTF-8 byte). Thus rejecting larger content loses no
// accepted encoding, and the authoritative decode can allocate only bounded
// password storage. There is deliberately no total-request-length cutoff.
fn bounded_password(data: &[u8]) -> bool {
    let Ok((_, offset)) = tags::decode_optional_context(data, 0, 0) else {
        return false;
    };
    let Ok((tag, pos)) = tags::decode_tag(data, offset) else {
        return false;
    };
    if !tag.is_context(1) {
        return false;
    }
    let Some(end) = pos
        .checked_add(tag.length as usize)
        .filter(|end| *end <= data.len())
    else {
        return false;
    };
    if end == data.len() {
        return true;
    }
    let Ok((password, _)) = tags::decode_optional_context(data, end, 2) else {
        return false;
    };
    match password {
        None => true,
        Some(content) => match content.first() {
            Some(0 | 5) => content.len() <= 21,
            Some(4) => content.len() <= 41,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::{primitives, tags::TagClass};
    use bytes::BytesMut;

    #[test]
    fn recovery_guard_removes_both_peer_maps_before_reuse() {
        use super::super::{Admission, RequestAdmissionPolicy};
        use crate::server::request_peer::canonical_requester;
        let admission = Admission::new(RequestAdmissionPolicy::default()).unwrap();
        for id in 0u32..2048 {
            let guard = admission
                .try_enter(
                    Class::Recovery,
                    canonical_requester(&id.to_be_bytes(), None),
                    false,
                )
                .unwrap();
            assert_eq!(admission.peer_entries(), [1, 0, 0]);
            assert_eq!(admission.recovery.peers.lock().unwrap().len(), 1);
            drop(guard);
            assert_eq!(admission.peer_entries(), [0; 3]);
            assert!(admission.recovery.peers.lock().unwrap().is_empty());
            assert_eq!(admission.recovery.permits.available_permits(), 4);
        }
    }

    fn assert_matches_decoder(data: &[u8]) {
        let expected = DeviceCommunicationControlRequest::decode(data)
            .is_ok_and(|r| r.enable_disable == EnableDisable::ENABLE);
        assert_eq!(
            matches!(
                confirmed_class(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL, data),
                Class::Recovery
            ),
            expected
        );
    }

    #[test]
    fn recovery_classifier_preserves_decoder_language_and_charset_byte_limits() {
        for duration in [None, Some(0), Some(u16::MAX)] {
            for mode in [
                EnableDisable::ENABLE,
                EnableDisable::DISABLE,
                EnableDisable::DISABLE_INITIATION,
                EnableDisable::from_raw(255),
            ] {
                for password in [
                    None,
                    Some(""),
                    Some("abcdefghijklmnopqrst"),
                    Some("éééééééééé"),
                    Some("abcdefghijklmnopqrstu"),
                ] {
                    let mut data = BytesMut::new();
                    DeviceCommunicationControlRequest {
                        time_duration: duration,
                        enable_disable: mode,
                        password: password.map(str::to_owned),
                    }
                    .encode(&mut data)
                    .unwrap();
                    assert_matches_decoder(&data);
                    data.extend_from_slice(&[0x39, 7, 0xff]); // Existing unrelated trailing tag tolerance.
                    assert_matches_decoder(&data);
                }
            }
        }
        for charset in 0..=6 {
            for len in 0..=44 {
                for byte in [0, b'a', 0x80, 0xd8, 0xff] {
                    let mut data = BytesMut::from(&[0x19, 0][..]);
                    tags::encode_tag(&mut data, 2, TagClass::Context, len + 1);
                    data.extend_from_slice(&[charset]);
                    data.extend_from_slice(&vec![byte; len as usize]);
                    assert_matches_decoder(&data);
                    // All truncated prefixes must also agree.
                    for end in 0..data.len() {
                        assert_matches_decoder(&data[..end]);
                    }
                }
            }
        }
        for duration in [65536, u64::MAX] {
            let mut data = BytesMut::new();
            primitives::encode_ctx_unsigned(&mut data, 0, duration);
            data.extend_from_slice(&[0x19, 0]);
            assert_matches_decoder(&data);
        }
        for data in [
            &[0x19, 0][..],
            &[0x1d, 2, 0, 0],
            &[0x19, 0, 0xff],
            &[0x19, 0, 0x2e],
        ] {
            assert_matches_decoder(data);
            assert!(matches!(
                confirmed_class(ConfirmedServiceChoice::REINITIALIZE_DEVICE, data),
                Class::Confirmed
            ));
        }
    }

    #[test]
    fn recovery_classifier_preflight_rejects_large_password_not_large_trailing_data() {
        for charset in [0, 4, 5] {
            let mut data = BytesMut::from(&[0x19, 0][..]);
            tags::encode_tag(&mut data, 2, TagClass::Context, 1_000_001);
            data.extend_from_slice(&[charset]);
            data.resize(data.len() + 1_000_000, b'a');
            assert!(!bounded_password(&data)); // No allocating character decoder reached.
            assert!(matches!(
                confirmed_class(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL, &data),
                Class::Confirmed
            ));
        }
        let mut trailing = vec![0x19, 0, 0x39, 1];
        trailing.resize(1_000_000, 0xff);
        assert!(bounded_password(&trailing));
        assert_matches_decoder(&trailing);
        assert!(matches!(
            confirmed_class(
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                &trailing
            ),
            Class::Recovery
        ));
    }
}
