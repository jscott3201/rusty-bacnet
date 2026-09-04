use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_objects::audit::CompletedAuditReceipt;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

const KEY_FORMAT_VERSION: u8 = 1;

pub(super) fn completed_receipt(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    request: &ConfirmedRequest,
    completed_at_unix_millis: u64,
) -> Result<CompletedAuditReceipt, Error> {
    let mut key = Vec::with_capacity(request.service_request.len() + 64);
    key.extend_from_slice(b"RBACR");
    key.push(KEY_FORMAT_VERSION);
    append_requester(&mut key, source_mac, source_network)?;
    key.push(u8::from(request.segmented));
    key.push(u8::from(request.more_follows));
    key.push(u8::from(request.segmented_response_accepted));
    append_optional_u8(&mut key, request.max_segments);
    key.extend_from_slice(&request.max_apdu_length.to_be_bytes());
    key.push(request.invoke_id);
    append_optional_u8(&mut key, request.sequence_number);
    append_optional_u8(&mut key, request.proposed_window_size);
    key.extend_from_slice(&request.service_choice.to_raw().to_be_bytes());
    append_bytes(&mut key, &request.service_request)?;
    CompletedAuditReceipt::new(key, completed_at_unix_millis)
}

fn append_requester(
    key: &mut Vec<u8>,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
) -> Result<(), Error> {
    if let Some(source) = source_network
        .filter(|source| (1..=0xfffe).contains(&source.network) && !source.mac_address.is_empty())
    {
        key.push(1);
        key.extend_from_slice(&source.network.to_be_bytes());
        append_bytes(key, &source.mac_address)
    } else {
        key.push(0);
        append_bytes(key, &MacAddr::from_slice(source_mac))
    }
}

fn append_optional_u8(key: &mut Vec<u8>, value: Option<u8>) {
    match value {
        Some(value) => key.extend_from_slice(&[1, value]),
        None => key.push(0),
    }
}

fn append_bytes(key: &mut Vec<u8>, value: &[u8]) -> Result<(), Error> {
    let length = u32::try_from(value.len())
        .map_err(|_| Error::OutOfRange("Audit receipt identity length exceeds u32".into()))?;
    key.extend_from_slice(&length.to_be_bytes());
    key.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bytes::Bytes;

    use super::*;

    fn request() -> ConfirmedRequest {
        ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: Some(4),
            max_apdu_length: 480,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: Bytes::from_static(b"payload"),
        }
    }

    fn key(request: &ConfirmedRequest) -> Vec<u8> {
        completed_receipt(b"peer", None, request, 1)
            .unwrap()
            .key()
            .to_vec()
    }

    #[test]
    fn exact_key_discriminates_every_confirmed_request_field() {
        let baseline = request();
        let expected = key(&baseline);
        let mut changed = Vec::new();

        let mut request = baseline.clone();
        request.segmented = true;
        changed.push(request);
        let mut request = baseline.clone();
        request.more_follows = true;
        changed.push(request);
        let mut request = baseline.clone();
        request.segmented_response_accepted = false;
        changed.push(request);
        let mut request = baseline.clone();
        request.max_segments = None;
        changed.push(request);
        let mut request = baseline.clone();
        request.max_apdu_length = 1024;
        changed.push(request);
        let mut request = baseline.clone();
        request.invoke_id = 8;
        changed.push(request);
        let mut request = baseline.clone();
        request.sequence_number = Some(0);
        changed.push(request);
        let mut request = baseline.clone();
        request.proposed_window_size = Some(1);
        changed.push(request);
        let mut request = baseline.clone();
        request.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
        changed.push(request);
        let mut request = baseline.clone();
        request.service_request = Bytes::from_static(b"changed");
        changed.push(request);

        for request in changed {
            assert_ne!(key(&request), expected);
        }
    }

    #[test]
    fn canonical_requester_matches_direct_and_valid_routed_tracker_policy() {
        let request = request();
        let direct = completed_receipt(b"router-a", None, &request, 1).unwrap();
        let other_direct = completed_receipt(b"router-b", None, &request, 1).unwrap();
        assert_ne!(direct.key(), other_direct.key());

        let origin = NpduAddress {
            network: 5,
            mac_address: MacAddr::from_slice(b"origin"),
        };
        let routed_a = completed_receipt(b"router-a", Some(&origin), &request, 1).unwrap();
        let routed_b = completed_receipt(b"router-b", Some(&origin), &request, 1).unwrap();
        assert_eq!(routed_a.key(), routed_b.key());
        assert_ne!(routed_a.key(), direct.key());
        let other_origin = NpduAddress {
            network: 6,
            mac_address: MacAddr::from_slice(b"origin"),
        };
        assert_ne!(
            routed_a.key(),
            completed_receipt(b"router-a", Some(&other_origin), &request, 1)
                .unwrap()
                .key()
        );

        let invalid = NpduAddress {
            network: 0,
            mac_address: MacAddr::from_slice(b"origin"),
        };
        let fallback = completed_receipt(b"router-a", Some(&invalid), &request, 1).unwrap();
        assert_eq!(fallback.key(), direct.key());
    }
}
