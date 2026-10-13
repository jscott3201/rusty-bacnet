//! Encode an arbitrary B/IPv6 message (Annex U) through every sink, then
//! decode it.
//!
//! Oracle: each message has the octets its Annex U.2 layout gives, built
//! here field by field: type 0x82, the function, a Length field counting the
//! whole message, the first VMAC, then the rest of the payload, with each
//! B/IPv6 address as its 16 IPv6 octets and its port. Encoding refuses a
//! BVLC-Result whose code U.2.1.1 doesn't define, then a message longer than
//! the Length field can state; inputs reach that limit from both sides.
//! Every sink with room holds exactly the expected octets, a fixed sink
//! without room refuses with `BufferExhausted`, and a refused encode leaves
//! every sink as it was. Each successful encode decodes back to an equal
//! message.

#![no_main]

use std::net::Ipv6Addr;

use arbitrary::Arbitrary;
use bacnet_bip_codec::ipv6::{
    decode, Bip6Address, Bip6Vmac, Bvlc6ResultCode, Bvll6, Bvll6Error, Bvll6Frame,
};
use bacnet_bip_codec::BvllSink;
use bip_codec_fuzz::{bvll_octets, check_sinks, Npdu, SinkKnobs, HEADER, MAX_LENGTH, TYPE_BIP6};
use libfuzzer_sys::fuzz_target;

/// The result codes Annex U.2.1.1 defines.
const RESULT_CODES: [u16; 6] = [0x0000, 0x0030, 0x0060, 0x0090, 0x00A0, 0x00C0];

type Vmac = [u8; 3];
type Address = ([u8; 16], u16);

#[derive(Debug, Arbitrary)]
enum Spec {
    Result(Vmac, u16),
    Unicast(Vmac, Vmac, Npdu),
    Broadcast(Vmac, Npdu),
    AddressResolution(Vmac, Vmac),
    ForwardedAddressResolution(Vmac, Vmac, Address),
    AddressResolutionAck(Vmac, Vmac),
    VirtualAddressResolution(Vmac),
    VirtualAddressResolutionAck(Vmac, Vmac),
    Forwarded(Vmac, Address, Npdu),
    Register(Vmac, u16),
    Delete(Vmac, Address),
    Distribute(Vmac, Npdu),
}

#[derive(Debug, Arbitrary)]
struct Input {
    spec: Spec,
    sinks: SinkKnobs,
}

fn address((ip, port): Address) -> Bip6Address {
    Bip6Address::new(Ipv6Addr::from(ip), port)
}

fn address_octets((ip, port): Address) -> Vec<u8> {
    [&ip[..], &port.to_be_bytes()].concat()
}

/// The message a spec describes, borrowing `npdu`, with its function code
/// and payload. The NPDU of a long spec is sized after the fields before it.
fn layout(spec: &Spec, npdu: &mut Vec<u8>) -> (u8, Vec<u8>) {
    match spec {
        Spec::Result(source, code) => (0x00, [&source[..], &code.to_be_bytes()].concat()),
        Spec::Unicast(source, destination, octets) => {
            *npdu = octets.octets(6);
            (0x01, [&source[..], destination, npdu].concat())
        }
        Spec::Broadcast(source, octets) => {
            *npdu = octets.octets(3);
            (0x02, [&source[..], npdu].concat())
        }
        Spec::AddressResolution(source, target) => (0x03, [*source, *target].concat()),
        Spec::ForwardedAddressResolution(source, target, original) => (
            0x04,
            [&source[..], target, &address_octets(*original)].concat(),
        ),
        Spec::AddressResolutionAck(source, destination) => (0x05, [*source, *destination].concat()),
        Spec::VirtualAddressResolution(source) => (0x06, source.to_vec()),
        Spec::VirtualAddressResolutionAck(source, destination) => {
            (0x07, [*source, *destination].concat())
        }
        Spec::Forwarded(source, original, octets) => {
            *npdu = octets.octets(21);
            (
                0x08,
                [&source[..], &address_octets(*original), npdu].concat(),
            )
        }
        Spec::Register(source, ttl) => (0x09, [&source[..], &ttl.to_be_bytes()].concat()),
        Spec::Delete(source, entry) => (0x0A, [&source[..], &address_octets(*entry)].concat()),
        Spec::Distribute(source, octets) => {
            *npdu = octets.octets(3);
            (0x0C, [&source[..], npdu].concat())
        }
    }
}

fn build<'a>(spec: &Spec, npdu: &'a [u8]) -> Bvll6<'a> {
    let vmac = |octets: &Vmac| Bip6Vmac::new(*octets);
    match spec {
        Spec::Result(source, code) => Bvll6::BvlcResult {
            source: vmac(source),
            code: Bvlc6ResultCode::from_raw(*code),
        },
        Spec::Unicast(source, destination, _) => Bvll6::OriginalUnicastNpdu {
            source: vmac(source),
            destination: vmac(destination),
            npdu,
        },
        Spec::Broadcast(source, _) => Bvll6::OriginalBroadcastNpdu {
            source: vmac(source),
            npdu,
        },
        Spec::AddressResolution(source, target) => Bvll6::AddressResolution {
            source: vmac(source),
            target: vmac(target),
        },
        Spec::ForwardedAddressResolution(source, target, original) => {
            Bvll6::ForwardedAddressResolution {
                original_source: vmac(source),
                target: vmac(target),
                original_address: address(*original),
            }
        }
        Spec::AddressResolutionAck(source, destination) => Bvll6::AddressResolutionAck {
            source: vmac(source),
            destination: vmac(destination),
        },
        Spec::VirtualAddressResolution(source) => Bvll6::VirtualAddressResolution {
            source: vmac(source),
        },
        Spec::VirtualAddressResolutionAck(source, destination) => {
            Bvll6::VirtualAddressResolutionAck {
                source: vmac(source),
                destination: vmac(destination),
            }
        }
        Spec::Forwarded(source, original, _) => Bvll6::ForwardedNpdu {
            original_source: vmac(source),
            original_address: address(*original),
            npdu,
        },
        Spec::Register(source, ttl) => Bvll6::RegisterForeignDevice {
            source: vmac(source),
            ttl: *ttl,
        },
        Spec::Delete(source, entry) => Bvll6::DeleteForeignDeviceTableEntry {
            source: vmac(source),
            entry: address(*entry),
        },
        Spec::Distribute(source, _) => Bvll6::DistributeBroadcastToNetwork {
            original_source: vmac(source),
            npdu,
        },
    }
}

fuzz_target!(|input: Input| {
    let mut npdu = Vec::new();
    let (function, payload) = layout(&input.spec, &mut npdu);
    let message = build(&input.spec, &npdu);
    assert_eq!(message.function().to_raw(), function);
    let wire = bvll_octets(TYPE_BIP6, function, &payload);
    let expected = match input.spec {
        Spec::Result(_, code) if !RESULT_CODES.contains(&code) => {
            Err(Bvll6Error::UndefinedResultCode { code })
        }
        _ if wire.len() > MAX_LENGTH => Err(Bvll6Error::TooLong { length: wire.len() }),
        _ => Ok(wire.clone()),
    };
    assert_eq!(
        message.encoded_len(),
        expected.as_ref().map(Vec::len).map_err(|error| *error)
    );

    let outputs = check_sinks(
        &expected,
        wire.len(),
        &input.sinks,
        |needed, available| Bvll6Error::BufferExhausted { needed, available },
        |output| message.encode_into(output),
        |sink: &mut dyn BvllSink| message.encode_to_sink(sink),
    );
    for output in outputs {
        assert_eq!(decode(&output), Ok(message));
        let frame = Bvll6Frame::decode(&output).expect("an encoded message decodes");
        assert_eq!(frame.function.to_raw(), function);
        assert_eq!(frame.payload, &output[HEADER..]);
    }
});
