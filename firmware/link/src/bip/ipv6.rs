//! The B/IP image's B/IPv6 part: `bacnet_bip_codec::ipv6` (Annex U).
//!
//! [`run`] drives the B/IPv6 codec through every message it knows, in the
//! same storage as the B/IPv4 part, and returns a [`Summary`]:
//!
//! - **Messages:** encode one message of each [`Bvll6`] variant into a slice
//!   and into the `heapless::Vec` sink, and decode each back.
//! - **Golden octets:** decode three datagrams written by hand from the
//!   Annex U.2 layouts.
//! - **Accessors:** the decoded Forwarded-NPDU's originator and NPDU, and
//!   how many messages name a destination VMAC.
//! - **Functions:** the payload shape of a few functions, and which NAK
//!   answers a few others.
//! - **Rejections:** a trailing octet, a B/IP type octet, a payload of the
//!   wrong length, the deleted function 0x0B, an undefined result code on
//!   encode, a short B/IPv6 address, and a message too long for a fixed
//!   slice and for a nearly full sink.

use core::net::Ipv6Addr;

use bacnet_bip_codec::ipv6::{
    decode, Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error,
};
use bacnet_bip_codec::PayloadShape;

use super::{Sink, NPDU, WIRE_LENGTH};
use crate::tally::{fnv1a, FNV_OFFSET};

/// This node's VMAC: device instance 1234.
pub const SOURCE: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
/// A peer's VMAC: device instance 5678.
pub const PEER: Bip6Vmac = Bip6Vmac::new([0x00, 0x16, 0x2E]);
/// A foreign device's random-instance VMAC (Annex H.7.2).
pub const RANDOM: Bip6Vmac = Bip6Vmac::new([0x4A, 0x5B, 0x6C]);
/// Where the forwarded NPDU started.
pub const ORIGIN: Bip6Address =
    Bip6Address::new(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 0x77), 0xBAC0);
/// A registered foreign device, on a non-default port.
pub const FOREIGN: Bip6Address =
    Bip6Address::new(Ipv6Addr::new(0x2001, 0xDB8, 1, 0, 0, 0, 0, 0x20), 47809);

/// One message of each variant, in function-code order.
pub const MESSAGES: [Bvll6<'static>; 12] = [
    Bvll6::BvlcResult {
        source: SOURCE,
        code: Bvlc6ResultCode::SUCCESSFUL_COMPLETION,
    },
    Bvll6::OriginalUnicastNpdu {
        source: SOURCE,
        destination: PEER,
        npdu: &NPDU,
    },
    Bvll6::OriginalBroadcastNpdu {
        source: SOURCE,
        npdu: &NPDU,
    },
    Bvll6::AddressResolution {
        source: SOURCE,
        target: PEER,
    },
    Bvll6::ForwardedAddressResolution {
        original_source: SOURCE,
        target: PEER,
        original_address: FOREIGN,
    },
    Bvll6::AddressResolutionAck {
        source: PEER,
        destination: SOURCE,
    },
    Bvll6::VirtualAddressResolution { source: SOURCE },
    Bvll6::VirtualAddressResolutionAck {
        source: PEER,
        destination: SOURCE,
    },
    Bvll6::ForwardedNpdu {
        original_source: SOURCE,
        original_address: ORIGIN,
        npdu: &NPDU,
    },
    Bvll6::RegisterForeignDevice {
        source: SOURCE,
        ttl: 300,
    },
    Bvll6::DeleteForeignDeviceTableEntry {
        source: SOURCE,
        entry: FOREIGN,
    },
    Bvll6::DistributeBroadcastToNetwork {
        original_source: RANDOM,
        npdu: &NPDU,
    },
];

/// Datagrams written by hand from the format subclauses of Annex U.2,
/// rather than by the encoder: the 4-octet BVLL header (type 0x82, the
/// function, then the whole message's length, high octet first), then the
/// function's fields, each starting with a VMAC.
pub const GOLDEN: [&[u8]; 3] = [
    // Register-Foreign-Device (U.2.10.1): VMAC 00 04 D2, then a
    // time-to-live of 300 s, 0x012C.
    &[0x82, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x01, 0x2C],
    // Forwarded-NPDU (U.2.9.1): VMAC 00 04 D2, the originator's address
    // [2001:db8::14] on UDP port 47809 (0xBAC1), then the NPDU 01 00.
    &[
        0x82, 0x08, 0x00, 0x1B, 0x00, 0x04, 0xD2, 0x20, 0x01, 0x0D, 0xB8, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0xBA, 0xC1, 0x01, 0x00,
    ],
    // Address-Resolution-ACK (U.2.6.1): from VMAC 00 16 2E to the asker,
    // VMAC 00 04 D2.
    &[0x82, 0x05, 0x00, 0x0A, 0x00, 0x16, 0x2E, 0x00, 0x04, 0xD2],
];

/// Functions whose payload shape the scenario asks for; 0x0B is unknown.
pub const SHAPED: [Bvlc6Function; 4] = [
    Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
    Bvlc6Function::FORWARDED_NPDU,
    Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
    Bvlc6Function::from_raw(0x0B),
];
/// Functions whose NAK the scenario asks for.
pub const NAKED: [Bvlc6Function; 3] = [
    Bvlc6Function::ADDRESS_RESOLUTION,
    Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK,
    Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION,
];
/// The rejection paths the scenario takes.
pub const REJECTIONS: usize = 8;

/// What [`run`] saw. Every field is plain data, so the host test can compare
/// the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Messages encoded into a slice.
    pub encoded: u8,
    /// Octets those messages took, in all.
    pub encoded_octets: u16,
    /// 32-bit FNV-1a over those octets in order.
    pub encoded_fnv: u32,
    /// Messages the sink took with the same octets as the slice.
    pub sink_matches: u8,
    /// Messages that decoded back equal to what was encoded.
    pub round_trips: u8,
    /// One bit per function code decoded: bit `n` for code `n`.
    pub functions: u16,
    /// Decoded messages that name a destination VMAC.
    pub destinations: u8,
    /// What each datagram in [`GOLDEN`] decodes to.
    pub goldens: [Option<Bvll6<'static>>; GOLDEN.len()],
    /// The decoded Forwarded-NPDU's originator.
    pub origin: Option<Bip6Address>,
    /// The decoded Forwarded-NPDU's NPDU length.
    pub forwarded_npdu: Option<usize>,
    /// The payload shape of each function in [`SHAPED`].
    pub shapes: [Option<PayloadShape>; SHAPED.len()],
    /// The NAK of each function in [`NAKED`].
    pub naks: [Option<Bvlc6ResultCode>; NAKED.len()],
    /// The error each rejection path produced: a trailing octet, type 0x81,
    /// a 1-octet time-to-live, function 0x0B, the result code 0x0010 on
    /// encode, a 6-octet address, and the 33-octet Forwarded-NPDU encoded
    /// into a 10-octet slice and into a sink with 14 octets left.
    pub rejections: [Option<Bvll6Error>; REJECTIONS],
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

impl Summary {
    pub(super) const fn new() -> Self {
        Self {
            encoded: 0,
            encoded_octets: 0,
            encoded_fnv: FNV_OFFSET,
            sink_matches: 0,
            round_trips: 0,
            functions: 0,
            destinations: 0,
            goldens: [None; GOLDEN.len()],
            origin: None,
            forwarded_npdu: None,
            shapes: [None; SHAPED.len()],
            naks: [None; NAKED.len()],
            rejections: [None; REJECTIONS],
            errors: 0,
        }
    }

    /// Count a call that failed where the scenario expects success.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the B/IPv6 part in the B/IP part's storage.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on.
pub fn run(wire: &mut [u8], sink: &mut Sink) -> Summary {
    let mut summary = Summary::new();
    for message in &MESSAGES {
        round_trip(message, wire, sink, &mut summary);
    }
    for (golden, datagram) in summary.goldens.iter_mut().zip(GOLDEN) {
        *golden = decode(datagram).ok();
    }
    for (shape, function) in summary.shapes.iter_mut().zip(SHAPED) {
        *shape = function.payload_shape();
    }
    for (nak, function) in summary.naks.iter_mut().zip(NAKED) {
        *nak = function.nak();
    }
    summary.rejections = rejections(wire, sink);
    summary
}

/// Encode `message` into `wire` and into `sink`, decode it from `wire`, and
/// read what the decoded message carries.
fn round_trip(message: &Bvll6<'_>, wire: &mut [u8], sink: &mut Sink, summary: &mut Summary) {
    let Ok(length) = message.encode_into(wire) else {
        summary.error();
        return;
    };
    let Some(octets) = wire.get(..length) else {
        summary.error();
        return;
    };
    summary.encoded += 1;
    summary.encoded_octets = summary
        .encoded_octets
        .saturating_add(u16::try_from(length).unwrap_or(u16::MAX));
    summary.encoded_fnv = fnv1a(summary.encoded_fnv, octets);
    sink.clear();
    match message.encode_to_sink(sink) {
        Ok(written) if written == length && sink.as_slice() == octets => {
            summary.sink_matches += 1;
        }
        _ => summary.error(),
    }
    let Ok(decoded) = decode(octets) else {
        summary.error();
        return;
    };
    if decoded == *message {
        summary.round_trips += 1;
    } else {
        summary.error();
    }
    summary.functions |= 1u16
        .checked_shl(u32::from(decoded.function().to_raw()))
        .unwrap_or(0);
    if decoded.destination_vmac().is_some() {
        summary.destinations += 1;
    }
    if let Bvll6::ForwardedNpdu { .. } = decoded {
        summary.origin = decoded.origin();
        summary.forwarded_npdu = decoded.npdu().map(<[u8]>::len);
    }
}

/// Take each rejection path and return the errors, `None` where a call
/// unexpectedly succeeded.
fn rejections(wire: &mut [u8], sink: &mut Sink) -> [Option<Bvll6Error>; REJECTIONS] {
    // A Register-Foreign-Device followed by one octet its Length field
    // doesn't count.
    let trailing = [0x82, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x01, 0x2C, 0x00];
    // The same message with the B/IP type octet.
    let bip = [0x81, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x01, 0x2C];
    // A Register-Foreign-Device whose time-to-live is one octet short.
    let short_ttl = [0x82, 0x09, 0x00, 0x08, 0x00, 0x04, 0xD2, 0x01];
    // Function 0x0B, whose clause this edition deletes.
    let deleted = [0x82, 0x0B, 0x00, 0x07, 0x00, 0x04, 0xD2];
    // 0x0010 is a B/IP result code, not a B/IPv6 one.
    let undefined_result = Bvll6::BvlcResult {
        source: SOURCE,
        code: Bvlc6ResultCode::from_raw(0x0010),
    };
    // A B/IP address's six octets where a B/IPv6 address's 18 belong.
    let short_address = [10, 0, 0, 1, 0xBA, 0xC0];
    // The Forwarded-NPDU of 33 octets.
    let forwarded = MESSAGES[8];
    // The sink, 50 octets into its 64.
    sink.clear();
    let crowded = sink.extend_from_slice(&[0; WIRE_LENGTH - 14]).is_ok();
    [
        decode(&trailing).err(),
        decode(&bip).err(),
        decode(&short_ttl).err(),
        decode(&deleted).err(),
        undefined_result.encode_into(wire).err(),
        Bip6Address::from_slice(&short_address).err(),
        wire.get_mut(..10)
            .and_then(|short| forwarded.encode_into(short).err()),
        forwarded.encode_to_sink(sink).err().filter(|_| crowded),
    ]
}
