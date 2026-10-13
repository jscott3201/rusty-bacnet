//! Every way the B/IPv6 decoder and encoder refuse a message, and the sinks.

use core::net::Ipv6Addr;

use bacnet_bip_codec::ipv6::{
    decode, Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error, Bvll6Frame,
};
use bacnet_bip_codec::{BvllSink, PayloadShape, SinkFull, MAX_BVLL_LENGTH};

const SOURCE: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
const ORIGIN: Bip6Address = Bip6Address::new(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 1), 47808);

/// A header with `function` and a Length field counting `payload`, then the
/// payload: well formed as a frame, whatever the payload holds.
fn frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).unwrap().to_be_bytes();
    let mut wire = vec![0x82, function, length[0], length[1]];
    wire.extend_from_slice(payload);
    wire
}

#[test]
fn shorter_than_the_header() {
    for length in 0..=3 {
        let wire = &[0x82, 0x01, 0x00][..length];
        assert_eq!(
            Bvll6Frame::decode(wire),
            Err(Bvll6Error::TooShort { length: wire.len() })
        );
        assert_eq!(decode(wire), Err(Bvll6Error::TooShort { length }));
    }
}

#[test]
fn not_a_bip6_message() {
    // 0x81 is the B/IP type (Annex J); 0x00 and 0x83 are nothing.
    for bvlc_type in [0x00, 0x81, 0x83, 0xFF] {
        assert_eq!(
            decode(&[bvlc_type, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2]),
            Err(Bvll6Error::NotBip6 { bvlc_type })
        );
    }
}

#[test]
fn length_field_must_equal_the_datagram() {
    // Declared longer than the datagram: truncated.
    assert_eq!(
        decode(&[0x82, 0x06, 0x00, 0x0A, 0x00, 0x04, 0xD2]),
        Err(Bvll6Error::LengthMismatch {
            declared: 10,
            received: 7
        })
    );
    // Declared shorter: trailing octets are refused, not ignored.
    assert_eq!(
        decode(&[0x82, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2, 0xDE, 0xAD]),
        Err(Bvll6Error::LengthMismatch {
            declared: 7,
            received: 9
        })
    );
    // A length below the header's own four octets can never match.
    for declared in 0..4 {
        assert_eq!(
            decode(&[0x82, 0x06, 0x00, declared]),
            Err(Bvll6Error::LengthMismatch {
                declared: usize::from(declared),
                received: 4
            })
        );
    }
    // The high octet counts: 0x0107 is 263, not 7.
    let mut long = frame(0x06, &[0x00, 0x04, 0xD2]);
    long[2] = 0x01;
    assert_eq!(
        decode(&long),
        Err(Bvll6Error::LengthMismatch {
            declared: 263,
            received: 7
        })
    );
}

#[test]
fn unknown_functions_have_frames_but_no_message() {
    // 0x0B is the deleted U.2.12; the rest were never defined.
    for function in [0x0B, 0x0D, 0x0F, 0x7F, 0xFF] {
        for payload in [&[][..], &[0x00, 0x04, 0xD2], &[0xAA; 30]] {
            let wire = frame(function, payload);
            let frame = Bvll6Frame::decode(&wire).unwrap();
            assert_eq!(frame.function, Bvlc6Function::from_raw(function));
            assert_eq!(frame.payload, payload);
            assert_eq!(
                frame.message(),
                Err(Bvll6Error::UnknownFunction { function })
            );
            assert_eq!(decode(&wire), Err(Bvll6Error::UnknownFunction { function }));
        }
    }
}

#[test]
fn payloads_of_the_wrong_length() {
    let cases: [(Bvlc6Function, &[usize]); 12] = [
        (Bvlc6Function::BVLC_RESULT, &[0, 2, 3, 4, 6, 7]),
        (Bvlc6Function::ORIGINAL_UNICAST_NPDU, &[0, 2, 3, 5]),
        (Bvlc6Function::ORIGINAL_BROADCAST_NPDU, &[0, 1, 2]),
        (Bvlc6Function::ADDRESS_RESOLUTION, &[0, 3, 5, 7, 9]),
        (
            Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION,
            &[0, 6, 21, 23, 25],
        ),
        (Bvlc6Function::ADDRESS_RESOLUTION_ACK, &[3, 5, 7]),
        (Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION, &[0, 2, 4, 6]),
        (Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK, &[3, 5, 7]),
        (Bvlc6Function::FORWARDED_NPDU, &[0, 3, 18, 20]),
        (Bvlc6Function::REGISTER_FOREIGN_DEVICE, &[0, 3, 4, 6]),
        (
            Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            &[0, 3, 18, 20, 22],
        ),
        (Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK, &[0, 1, 2]),
    ];
    for (function, lengths) in cases {
        for &length in lengths {
            let wire = frame(function.to_raw(), &vec![0; length]);
            assert_eq!(
                decode(&wire),
                Err(Bvll6Error::PayloadLength { function, length }),
                "{function} with {length} octets"
            );
        }
    }
}

/// `payload_shape` and the decoder agree on every function and length.
#[test]
fn payload_shape_matches_the_decoder() {
    for raw in 0..=0xFF {
        let function = Bvlc6Function::from_raw(raw);
        for length in 0..=48 {
            let wire = frame(raw, &vec![0x01; length]);
            let decoded = decode(&wire);
            match function.payload_shape() {
                Some(shape) => {
                    assert!(!matches!(shape, PayloadShape::Table { .. }));
                    assert_eq!(
                        decoded.is_ok(),
                        shape.admits(length),
                        "{function:?} {length}: {decoded:?}"
                    );
                }
                None => assert_eq!(decoded, Err(Bvll6Error::UnknownFunction { function: raw })),
            }
        }
    }
}

#[test]
fn addresses_of_the_wrong_length() {
    for length in [0, 6, 16, 17, 19, 24] {
        assert_eq!(
            Bip6Address::from_slice(&vec![0; length]),
            Err(Bvll6Error::AddressLength { length })
        );
    }
}

#[test]
fn encode_refuses_an_undefined_result_code() {
    let mut wire = [0u8; 9];
    // B/IP's 0x0010 and 0x0020 are not B/IPv6 codes.
    for code in [0x0001, 0x0010, 0x0020, 0x0031, 0xFFFF] {
        assert_eq!(
            Bvll6::BvlcResult {
                source: SOURCE,
                code: Bvlc6ResultCode::from_raw(code)
            }
            .encode_into(&mut wire),
            Err(Bvll6Error::UndefinedResultCode { code })
        );
    }
    assert_eq!(wire, [0; 9]);
}

#[test]
fn encode_refuses_more_than_the_length_field_can_state() {
    let npdu = vec![0x01; MAX_BVLL_LENGTH];
    let mut wire = vec![0u8; MAX_BVLL_LENGTH + 16];

    // The largest message fits exactly: the header and two VMACs, 10 octets.
    let largest = Bvll6::OriginalUnicastNpdu {
        source: SOURCE,
        destination: SOURCE,
        npdu: &npdu[..MAX_BVLL_LENGTH - 10],
    };
    assert_eq!(largest.encode_into(&mut wire), Ok(MAX_BVLL_LENGTH));
    assert_eq!(wire[2..4], [0xFF, 0xFF]);
    assert_eq!(decode(&wire[..MAX_BVLL_LENGTH]), Ok(largest));

    let too_long = Bvll6::OriginalUnicastNpdu {
        source: SOURCE,
        destination: SOURCE,
        npdu: &npdu[..MAX_BVLL_LENGTH - 9],
    };
    assert_eq!(
        too_long.encode_into(&mut wire),
        Err(Bvll6Error::TooLong {
            length: MAX_BVLL_LENGTH + 1
        })
    );
    // Each message's fixed fields count: 7 octets before a broadcast's or a
    // distribution's NPDU, 25 before a Forwarded-NPDU's.
    for (message, fixed) in [
        (
            Bvll6::OriginalBroadcastNpdu {
                source: SOURCE,
                npdu: &npdu[..MAX_BVLL_LENGTH - 6],
            },
            7,
        ),
        (
            Bvll6::DistributeBroadcastToNetwork {
                original_source: SOURCE,
                npdu: &npdu[..MAX_BVLL_LENGTH - 6],
            },
            7,
        ),
        (
            Bvll6::ForwardedNpdu {
                original_source: SOURCE,
                original_address: ORIGIN,
                npdu: &npdu[..MAX_BVLL_LENGTH - 24],
            },
            25,
        ),
    ] {
        assert_eq!(
            message.encoded_len(),
            Err(Bvll6Error::TooLong {
                length: MAX_BVLL_LENGTH + 1
            }),
            "{fixed}"
        );
    }
}

#[test]
fn a_fixed_sink_too_small_is_left_untouched() {
    let message = Bvll6::OriginalBroadcastNpdu {
        source: SOURCE,
        npdu: &[0x01, 0x00, 0x10, 0x08],
    };
    let mut wire = [0xEE; 10];
    assert_eq!(
        message.encode_into(&mut wire),
        Err(Bvll6Error::BufferExhausted {
            needed: 11,
            available: 10
        })
    );
    assert_eq!(wire, [0xEE; 10]);

    // The slice sink advances past what it wrote.
    let mut storage = [0u8; 13];
    let mut sink = &mut storage[..];
    assert_eq!(message.encode_to_sink(&mut sink), Ok(11));
    assert_eq!(sink.len(), 2);
    assert_eq!(BvllSink::remaining_capacity(&sink), Some(2));
    assert_eq!(
        sink.write(&[0; 3]),
        Err(SinkFull {
            needed: 3,
            available: 2
        })
    );
}

/// A sink that takes the header and then refuses: the encoder reports its
/// refusal as `BufferExhausted`.
#[test]
fn a_sink_that_refuses_mid_message() {
    struct Refuses(usize);
    impl BvllSink for Refuses {
        fn remaining_capacity(&self) -> Option<usize> {
            None
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
            if self.0 == 0 {
                return Err(SinkFull {
                    needed: bytes.len(),
                    available: 0,
                });
            }
            self.0 -= 1;
            Ok(())
        }
    }
    let message = Bvll6::ForwardedNpdu {
        original_source: SOURCE,
        original_address: ORIGIN,
        npdu: &[0x01, 0x00],
    };
    assert_eq!(
        message.encode_to_sink(&mut Refuses(1)),
        Err(Bvll6Error::BufferExhausted {
            needed: 18,
            available: 0
        })
    );
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_sink() {
    let message = Bvll6::RegisterForeignDevice {
        source: SOURCE,
        ttl: 300,
    };
    let mut sink = heapless::Vec::<u8, 9>::new();
    assert_eq!(message.encode_to_sink(&mut sink), Ok(9));
    assert_eq!(sink, [0x82, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x01, 0x2C]);
    assert_eq!(
        message.encode_to_sink(&mut sink),
        Err(Bvll6Error::BufferExhausted {
            needed: 9,
            available: 0
        })
    );
    assert_eq!(sink.len(), 9);
}

#[cfg(feature = "alloc")]
#[test]
fn vec_sink_grows() {
    let mut sink = vec![0xAA];
    let message = Bvll6::VirtualAddressResolution { source: SOURCE };
    assert_eq!(message.encode_to_sink(&mut sink), Ok(7));
    assert_eq!(sink, [0xAA, 0x82, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2]);
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_sink_grows() {
    let mut sink = bytes::BytesMut::new();
    let message = Bvll6::DeleteForeignDeviceTableEntry {
        source: SOURCE,
        entry: ORIGIN,
    };
    assert_eq!(message.encode_to_sink(&mut sink), Ok(25));
    assert_eq!(decode(&sink), Ok(message));
}

#[test]
fn errors_display_their_fields() {
    let cases = [
        (
            Bvll6Error::TooShort { length: 2 },
            "BVLC6 datagram too short: 2 octets",
        ),
        (Bvll6Error::NotBip6 { bvlc_type: 0x81 }, "0x81"),
        (
            Bvll6Error::LengthMismatch {
                declared: 7,
                received: 9,
            },
            "length 7",
        ),
        (
            Bvll6Error::UnknownFunction { function: 0x0B },
            "unknown BVLC6 function 0x0B",
        ),
        (
            Bvll6Error::PayloadLength {
                function: Bvlc6Function::REGISTER_FOREIGN_DEVICE,
                length: 4,
            },
            "REGISTER_FOREIGN_DEVICE cannot carry a 4-octet payload",
        ),
        (Bvll6Error::AddressLength { length: 6 }, "18 octets, got 6"),
        (
            Bvll6Error::TooLong { length: 65536 },
            "BVLC6 message of 65536 octets exceeds the 16-bit BVLC6 length field",
        ),
        (Bvll6Error::UndefinedResultCode { code: 0x10 }, "0x0010"),
        (
            Bvll6Error::BufferExhausted {
                needed: 11,
                available: 10,
            },
            "need 11, have 10",
        ),
    ];
    for (error, text) in cases {
        let shown = error.to_string();
        assert!(shown.contains(text), "{shown:?} lacks {text:?}");
        let _: &dyn core::error::Error = &error;
    }
    let full = SinkFull {
        needed: 3,
        available: 2,
    };
    assert!(full.to_string().contains("need 3, have 2"));
    let _: &dyn core::error::Error = &full;
}
