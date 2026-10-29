//! B/IPv6 property tests: decode never panics, every accepted datagram
//! re-encodes to itself, and every message the encoder writes decodes back,
//! only whole.

use core::net::Ipv6Addr;

use bacnet_bip_codec::ipv6::{
    Bip6Address, Bip6Vmac, Bvlc6ResultCode, Bvll6, Bvll6Error, Bvll6Frame, decode,
};
use proptest::prelude::*;

/// The owned ingredients of one message, which [`Spec::message`] borrows.
#[derive(Debug, Clone)]
enum Spec {
    Result(Bip6Vmac, u16),
    Unicast(Bip6Vmac, Bip6Vmac, Vec<u8>),
    Broadcast(Bip6Vmac, Vec<u8>),
    Resolution(Bip6Vmac, Bip6Vmac),
    ForwardedResolution(Bip6Vmac, Bip6Vmac, Bip6Address),
    ResolutionAck(Bip6Vmac, Bip6Vmac),
    Virtual(Bip6Vmac),
    VirtualAck(Bip6Vmac, Bip6Vmac),
    Forwarded(Bip6Vmac, Bip6Address, Vec<u8>),
    Register(Bip6Vmac, u16),
    Delete(Bip6Vmac, Bip6Address),
    Distribute(Bip6Vmac, Vec<u8>),
}

impl Spec {
    fn message(&self) -> Bvll6<'_> {
        match self {
            Self::Result(source, code) => Bvll6::BvlcResult {
                source: *source,
                code: Bvlc6ResultCode::from_raw(*code),
            },
            Self::Unicast(source, destination, npdu) => Bvll6::OriginalUnicastNpdu {
                source: *source,
                destination: *destination,
                npdu,
            },
            Self::Broadcast(source, npdu) => Bvll6::OriginalBroadcastNpdu {
                source: *source,
                npdu,
            },
            Self::Resolution(source, target) => Bvll6::AddressResolution {
                source: *source,
                target: *target,
            },
            Self::ForwardedResolution(original_source, target, original_address) => {
                Bvll6::ForwardedAddressResolution {
                    original_source: *original_source,
                    target: *target,
                    original_address: *original_address,
                }
            }
            Self::ResolutionAck(source, destination) => Bvll6::AddressResolutionAck {
                source: *source,
                destination: *destination,
            },
            Self::Virtual(source) => Bvll6::VirtualAddressResolution { source: *source },
            Self::VirtualAck(source, destination) => Bvll6::VirtualAddressResolutionAck {
                source: *source,
                destination: *destination,
            },
            Self::Forwarded(original_source, original_address, npdu) => Bvll6::ForwardedNpdu {
                original_source: *original_source,
                original_address: *original_address,
                npdu,
            },
            Self::Register(source, ttl) => Bvll6::RegisterForeignDevice {
                source: *source,
                ttl: *ttl,
            },
            Self::Delete(source, entry) => Bvll6::DeleteForeignDeviceTableEntry {
                source: *source,
                entry: *entry,
            },
            Self::Distribute(original_source, npdu) => Bvll6::DistributeBroadcastToNetwork {
                original_source: *original_source,
                npdu,
            },
        }
    }
}

fn vmac() -> impl Strategy<Value = Bip6Vmac> {
    any::<[u8; 3]>().prop_map(Bip6Vmac::new)
}

fn address() -> impl Strategy<Value = Bip6Address> {
    (any::<[u8; 16]>(), any::<u16>())
        .prop_map(|(ip, port)| Bip6Address::new(Ipv6Addr::from(ip), port))
}

fn npdu() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

fn spec() -> impl Strategy<Value = Spec> {
    let result_code = prop::sample::select(vec![0x0000u16, 0x30, 0x60, 0x90, 0xA0, 0xC0]);
    prop_oneof![
        (vmac(), result_code).prop_map(|(source, code)| Spec::Result(source, code)),
        (vmac(), vmac(), npdu()).prop_map(|(s, d, npdu)| Spec::Unicast(s, d, npdu)),
        (vmac(), npdu()).prop_map(|(source, npdu)| Spec::Broadcast(source, npdu)),
        (vmac(), vmac()).prop_map(|(source, target)| Spec::Resolution(source, target)),
        (vmac(), vmac(), address()).prop_map(|(s, t, a)| Spec::ForwardedResolution(s, t, a)),
        (vmac(), vmac()).prop_map(|(source, dest)| Spec::ResolutionAck(source, dest)),
        vmac().prop_map(Spec::Virtual),
        (vmac(), vmac()).prop_map(|(source, dest)| Spec::VirtualAck(source, dest)),
        (vmac(), address(), npdu()).prop_map(|(s, a, npdu)| Spec::Forwarded(s, a, npdu)),
        (vmac(), any::<u16>()).prop_map(|(source, ttl)| Spec::Register(source, ttl)),
        (vmac(), address()).prop_map(|(source, entry)| Spec::Delete(source, entry)),
        (vmac(), npdu()).prop_map(|(source, npdu)| Spec::Distribute(source, npdu)),
    ]
}

/// Arbitrary octets, mostly with a B/IPv6 header whose Length field
/// matches, so decoding gets past the header often enough to reach the
/// payloads.
fn datagram() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..80),
        (0u8..=0x10, prop::collection::vec(any::<u8>(), 0..80)).prop_map(|(function, payload)| {
            let length = u16::try_from(4 + payload.len()).unwrap().to_be_bytes();
            let mut wire = vec![0x82, function, length[0], length[1]];
            wire.extend_from_slice(&payload);
            wire
        }),
    ]
}

proptest! {
    #[test]
    fn decode_never_panics_and_reencodes_what_it_accepts(wire in datagram()) {
        let frame = Bvll6Frame::decode(&wire);
        match decode(&wire) {
            Ok(message) => {
                prop_assert_eq!(frame.unwrap().function, message.function());
                prop_assert_eq!(&wire[4..7], &message.source_vmac().octets()[..]);
                let mut out = vec![0u8; wire.len()];
                match message.encode_into(&mut out) {
                    Ok(length) => {
                        prop_assert_eq!(length, wire.len());
                        prop_assert_eq!(&out, &wire);
                    }
                    // The only thing decode passes that encode refuses.
                    Err(error) => prop_assert!(
                        matches!(error, Bvll6Error::UndefinedResultCode { .. }),
                        "{error:?}"
                    ),
                }
            }
            Err(error) => {
                // A refused header refuses the message for the same reason.
                if let Err(header) = frame {
                    prop_assert_eq!(header, error);
                }
            }
        }
    }

    #[test]
    fn encoded_messages_decode_whole_and_only_whole(spec in spec(), extra in any::<u8>()) {
        let message = spec.message();
        let mut wire = vec![0u8; message.encoded_len().unwrap()];
        let length = message.encode_into(&mut wire).unwrap();
        prop_assert_eq!(length, wire.len());
        prop_assert_eq!(decode(&wire), Ok(message));

        for end in 0..length {
            prop_assert!(decode(&wire[..end]).is_err());
        }
        wire.push(extra);
        prop_assert_eq!(
            decode(&wire),
            Err(Bvll6Error::LengthMismatch { declared: length, received: length + 1 })
        );
    }

    #[test]
    fn addresses_and_vmacs_round_trip(address in address(), vmac in vmac()) {
        let octets = address.to_octets();
        prop_assert_eq!(Bip6Address::from_octets(octets), address);
        prop_assert_eq!(Bip6Address::from_slice(&octets), Ok(address));
        prop_assert_eq!(&octets[..16], &address.ip.octets()[..]);
        prop_assert_eq!(u16::from_be_bytes([octets[16], octets[17]]), address.port);
        prop_assert_eq!(Bip6Vmac::new(vmac.octets()), vmac);
    }
}
