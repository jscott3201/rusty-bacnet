//! Property tests: decode never panics, every accepted datagram re-encodes
//! to itself, and every message the encoder writes decodes back, only whole.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{
    decode, BdtEntries, BdtEntry, BvlcResultCode, Bvll, BvllError, BvllFrame, FdtEntries, FdtEntry,
};
use proptest::prelude::*;

/// The owned ingredients of one message, which [`Spec::message`] borrows.
#[derive(Debug, Clone)]
enum Spec {
    Result(u16),
    WriteBdt(Vec<BdtEntry>),
    ReadBdt,
    ReadBdtAck(Vec<BdtEntry>),
    Forwarded(SocketAddrV4, Vec<u8>),
    Register(u16),
    ReadFdt,
    ReadFdtAck(Vec<FdtEntry>),
    Delete(SocketAddrV4),
    Distribute(Vec<u8>),
    Unicast(Vec<u8>),
    Broadcast(Vec<u8>),
}

impl Spec {
    fn message(&self) -> Bvll<'_> {
        match self {
            Self::Result(code) => Bvll::BvlcResult(BvlcResultCode::from_raw(*code)),
            Self::WriteBdt(rows) => {
                Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(rows))
            }
            Self::ReadBdt => Bvll::ReadBroadcastDistributionTable,
            Self::ReadBdtAck(rows) => {
                Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(rows))
            }
            Self::Forwarded(origin, npdu) => Bvll::ForwardedNpdu {
                origin: *origin,
                npdu,
            },
            Self::Register(ttl) => Bvll::RegisterForeignDevice { ttl: *ttl },
            Self::ReadFdt => Bvll::ReadForeignDeviceTable,
            Self::ReadFdtAck(rows) => {
                Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(rows))
            }
            Self::Delete(address) => Bvll::DeleteForeignDeviceTableEntry(*address),
            Self::Distribute(npdu) => Bvll::DistributeBroadcastToNetwork(npdu),
            Self::Unicast(npdu) => Bvll::OriginalUnicastNpdu(npdu),
            Self::Broadcast(npdu) => Bvll::OriginalBroadcastNpdu(npdu),
        }
    }
}

fn address() -> impl Strategy<Value = SocketAddrV4> {
    (any::<[u8; 4]>(), any::<u16>()).prop_map(|(ip, port)| SocketAddrV4::new(ip.into(), port))
}

fn bdt_entry() -> impl Strategy<Value = BdtEntry> {
    (address(), any::<[u8; 4]>()).prop_map(|(address, mask)| BdtEntry::new(address, mask.into()))
}

fn fdt_entry() -> impl Strategy<Value = FdtEntry> {
    (address(), any::<u16>(), any::<u16>())
        .prop_map(|(address, ttl, remaining)| FdtEntry::new(address, ttl, remaining))
}

fn npdu() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

fn spec() -> impl Strategy<Value = Spec> {
    let result_code = prop::sample::select(vec![0x0000u16, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
    prop_oneof![
        result_code.prop_map(Spec::Result),
        prop::collection::vec(bdt_entry(), 0..6).prop_map(Spec::WriteBdt),
        Just(Spec::ReadBdt),
        prop::collection::vec(bdt_entry(), 0..6).prop_map(Spec::ReadBdtAck),
        (address(), npdu()).prop_map(|(origin, npdu)| Spec::Forwarded(origin, npdu)),
        any::<u16>().prop_map(Spec::Register),
        Just(Spec::ReadFdt),
        prop::collection::vec(fdt_entry(), 0..6).prop_map(Spec::ReadFdtAck),
        address().prop_map(Spec::Delete),
        npdu().prop_map(Spec::Distribute),
        npdu().prop_map(Spec::Unicast),
        npdu().prop_map(Spec::Broadcast),
    ]
}

/// Arbitrary octets, mostly with a B/IP header whose Length field matches,
/// so decoding gets past the header often enough to reach the payloads.
fn datagram() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..80),
        (0u8..=0x10, prop::collection::vec(any::<u8>(), 0..80)).prop_map(|(function, payload)| {
            let length = u16::try_from(4 + payload.len()).unwrap().to_be_bytes();
            let mut wire = vec![0x81, function, length[0], length[1]];
            wire.extend_from_slice(&payload);
            wire
        }),
    ]
}

proptest! {
    #[test]
    fn decode_never_panics_and_reencodes_what_it_accepts(wire in datagram()) {
        let frame = BvllFrame::decode(&wire);
        match decode(&wire) {
            Ok(message) => {
                prop_assert_eq!(frame.unwrap().function, message.function());
                let mut out = vec![0u8; wire.len()];
                match message.encode_into(&mut out) {
                    Ok(length) => {
                        prop_assert_eq!(length, wire.len());
                        prop_assert_eq!(&out, &wire);
                    }
                    // The only thing decode passes that encode refuses.
                    Err(error) => prop_assert!(
                        matches!(error, BvllError::UndefinedResultCode { .. }),
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
            Err(BvllError::LengthMismatch { declared: length, received: length + 1 })
        );
    }

    #[test]
    fn table_entries_round_trip(entry in bdt_entry(), row in fdt_entry()) {
        prop_assert_eq!(BdtEntry::from_octets(entry.to_octets()), entry);
        prop_assert_eq!(FdtEntry::from_octets(row.to_octets()), row);
        let octets = entry.to_octets();
        prop_assert_eq!(&octets[..4], &entry.address.ip().octets()[..]);
        prop_assert_eq!(u16::from_be_bytes([octets[4], octets[5]]), entry.address.port());
        prop_assert_eq!(Ipv4Addr::new(octets[6], octets[7], octets[8], octets[9]), entry.mask);
    }
}
