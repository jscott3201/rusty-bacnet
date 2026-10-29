//! Every way decode and encode refuse a message, and the sinks.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{
    BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll, BvllError, BvllFrame, BvllSink,
    FdtEntries, MAX_BVLL_LENGTH, PayloadShape, SinkFull, address_from_slice, decode,
};

/// A header with `function` and a Length field counting `payload`, then the
/// payload: well formed as a frame, whatever the payload holds.
fn frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).unwrap().to_be_bytes();
    let mut wire = vec![0x81, function, length[0], length[1]];
    wire.extend_from_slice(payload);
    wire
}

#[test]
fn shorter_than_the_header() {
    for length in 0..=3 {
        let wire = &[0x81, 0x0A, 0x00][..length];
        assert_eq!(
            BvllFrame::decode(wire),
            Err(BvllError::TooShort { length: wire.len() })
        );
    }
}

#[test]
fn not_a_bip_message() {
    // 0x82 is the BACnet/IPv6 type (Annex U); 0x00 is nothing.
    for bvlc_type in [0x00, 0x80, 0x82, 0xFF] {
        assert_eq!(
            decode(&[bvlc_type, 0x0A, 0x00, 0x04]),
            Err(BvllError::NotBip { bvlc_type })
        );
    }
}

#[test]
fn length_field_must_equal_the_datagram() {
    // Declared longer than the datagram: truncated.
    assert_eq!(
        decode(&[0x81, 0x0A, 0x00, 0x64]),
        Err(BvllError::LengthMismatch {
            declared: 100,
            received: 4
        })
    );
    // Declared shorter: trailing octets are refused, not ignored.
    assert_eq!(
        decode(&[0x81, 0x0A, 0x00, 0x06, 0x01, 0x00, 0xDE, 0xAD]),
        Err(BvllError::LengthMismatch {
            declared: 6,
            received: 8
        })
    );
    // A length below the header's own four octets can never match.
    for declared in 0..4 {
        assert_eq!(
            decode(&[0x81, 0x02, 0x00, declared]),
            Err(BvllError::LengthMismatch {
                declared: usize::from(declared),
                received: 4
            })
        );
    }
}

#[test]
fn unknown_functions_have_frames_but_no_message() {
    for function in [0x0C, 0x0D, 0x7F, 0xFF] {
        let wire = frame(function, &[0xAA, 0xBB]);
        let frame = BvllFrame::decode(&wire).unwrap();
        assert_eq!(frame.function, BvlcFunction::from_raw(function));
        assert_eq!(frame.payload, [0xAA, 0xBB]);
        assert_eq!(
            frame.message(),
            Err(BvllError::UnknownFunction { function })
        );
        assert_eq!(decode(&wire), Err(BvllError::UnknownFunction { function }));
    }
}

#[test]
fn payloads_of_the_wrong_length() {
    let cases: [(BvlcFunction, &[usize]); 9] = [
        (BvlcFunction::BVLC_RESULT, &[0, 1, 3, 4]),
        (
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            &[1, 9, 11, 19],
        ),
        (BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, &[1, 2, 10]),
        (
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
            &[5, 15],
        ),
        (BvlcFunction::FORWARDED_NPDU, &[0, 1, 5]),
        (BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0, 1, 3]),
        (BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[1, 2]),
        (BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &[9, 15, 21]),
        (
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            &[0, 5, 7, 10],
        ),
    ];
    for (function, lengths) in cases {
        for &length in lengths {
            let wire = frame(function.to_raw(), &vec![0; length]);
            assert_eq!(
                decode(&wire),
                Err(BvllError::PayloadLength { function, length }),
                "{function} with {length} octets"
            );
        }
    }
}

/// `payload_shape` and the decoder agree on every function and length.
#[test]
fn payload_shape_matches_the_decoder() {
    for raw in 0..=0xFF {
        let function = BvlcFunction::from_raw(raw);
        for length in 0..=48 {
            let wire = frame(raw, &vec![0x01; length]);
            let decoded = decode(&wire);
            match function.payload_shape() {
                Some(shape) => assert_eq!(
                    decoded.is_ok(),
                    shape.admits(length),
                    "{function:?} {length}: {decoded:?}"
                ),
                None => assert_eq!(decoded, Err(BvllError::UnknownFunction { function: raw })),
            }
        }
    }
    assert!(PayloadShape::Table { entry_length: 10 }.admits(0));
    assert!(!PayloadShape::Npdu { prefix: 6 }.admits(5));
}

#[test]
fn tables_and_addresses_of_the_wrong_length() {
    assert_eq!(
        BdtEntries::from_wire(&[0; 11]).unwrap_err(),
        BvllError::TableLength {
            length: 11,
            entry_length: 10
        }
    );
    assert_eq!(FdtEntries::from_wire(&[0; 20]).unwrap().len(), 2);
    assert!(FdtEntries::from_wire(&[]).unwrap().is_empty());
    for length in [0, 5, 7, 18] {
        assert_eq!(
            address_from_slice(&vec![0; length]),
            Err(BvllError::AddressLength { length })
        );
    }
    assert_eq!(
        address_from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
        Ok(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 0xBAC0))
    );
}

#[test]
fn encode_refuses_an_undefined_result_code() {
    let mut wire = [0u8; 6];
    for code in [0x0001, 0x0070, 0xFFFF] {
        assert_eq!(
            Bvll::BvlcResult(BvlcResultCode::from_raw(code)).encode_into(&mut wire),
            Err(BvllError::UndefinedResultCode { code })
        );
    }
    assert_eq!(wire, [0; 6]);
}

#[test]
fn encode_refuses_more_than_the_length_field_can_state() {
    let npdu = vec![0x01; MAX_BVLL_LENGTH];
    let mut wire = vec![0u8; MAX_BVLL_LENGTH + 16];

    // The largest message fits exactly.
    let largest = Bvll::OriginalUnicastNpdu(&npdu[..MAX_BVLL_LENGTH - 4]);
    assert_eq!(largest.encode_into(&mut wire), Ok(MAX_BVLL_LENGTH));
    assert_eq!(wire[2..4], [0xFF, 0xFF]);
    assert_eq!(decode(&wire[..MAX_BVLL_LENGTH]), Ok(largest));

    let too_long = Bvll::OriginalUnicastNpdu(&npdu[..MAX_BVLL_LENGTH - 3]);
    assert_eq!(
        too_long.encode_into(&mut wire),
        Err(BvllError::TooLong {
            length: MAX_BVLL_LENGTH + 1
        })
    );
    // A Forwarded-NPDU's six address octets count too.
    let forwarded = Bvll::ForwardedNpdu {
        origin: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808),
        npdu: &npdu[..MAX_BVLL_LENGTH - 9],
    };
    assert_eq!(
        forwarded.encoded_len(),
        Err(BvllError::TooLong {
            length: MAX_BVLL_LENGTH + 1
        })
    );
    // So does a table: 6553 entries fit, 6554 don't.
    let entry = BdtEntry::new(
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808),
        Ipv4Addr::BROADCAST,
    );
    let rows = vec![entry; 6554];
    assert_eq!(
        Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows[..6553]))
            .encoded_len(),
        Ok(4 + 65530)
    );
    assert_eq!(
        Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&rows)).encoded_len(),
        Err(BvllError::TooLong { length: 4 + 65540 })
    );
}

#[test]
fn a_fixed_sink_too_small_is_left_untouched() {
    let message = Bvll::OriginalBroadcastNpdu(&[0x01, 0x00, 0x10, 0x08]);
    let mut wire = [0xEE; 7];
    assert_eq!(
        message.encode_into(&mut wire),
        Err(BvllError::BufferExhausted {
            needed: 8,
            available: 7
        })
    );
    assert_eq!(wire, [0xEE; 7]);

    // The slice sink advances past what it wrote.
    let mut storage = [0u8; 10];
    let mut sink = &mut storage[..];
    assert_eq!(message.encode_to_sink(&mut sink), Ok(8));
    assert_eq!(sink.len(), 2);
    assert_eq!(BvllSink::remaining_capacity(&sink), Some(2));
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
    let message = Bvll::ForwardedNpdu {
        origin: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 47808),
        npdu: &[0x01, 0x00],
    };
    // The header goes through; the 6-octet origin is refused.
    assert_eq!(
        message.encode_to_sink(&mut Refuses(1)),
        Err(BvllError::BufferExhausted {
            needed: 6,
            available: 0
        })
    );
    // A table is refused row by row: the first 10-octet entry.
    let rows = [BdtEntry::new(
        SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 47808),
        Ipv4Addr::BROADCAST,
    )];
    let table = Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&rows));
    assert_eq!(
        table.encode_to_sink(&mut Refuses(1)),
        Err(BvllError::BufferExhausted {
            needed: 10,
            available: 0
        })
    );
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_sink() {
    let message = Bvll::RegisterForeignDevice { ttl: 300 };
    let mut sink = heapless::Vec::<u8, 6>::new();
    assert_eq!(message.encode_to_sink(&mut sink), Ok(6));
    assert_eq!(sink, [0x81, 0x05, 0x00, 0x06, 0x01, 0x2C]);
    assert_eq!(
        message.encode_to_sink(&mut sink),
        Err(BvllError::BufferExhausted {
            needed: 6,
            available: 0
        })
    );
    assert_eq!(sink.len(), 6);
}

#[cfg(feature = "alloc")]
#[test]
fn vec_sink_grows() {
    let mut sink = vec![0xAA];
    assert_eq!(
        Bvll::ReadForeignDeviceTable.encode_to_sink(&mut sink),
        Ok(4)
    );
    assert_eq!(sink, [0xAA, 0x81, 0x06, 0x00, 0x04]);
}

#[cfg(feature = "bytes")]
#[test]
fn bytes_sink_grows() {
    let mut sink = bytes::BytesMut::new();
    let message =
        Bvll::DeleteForeignDeviceTableEntry(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 47809));
    assert_eq!(message.encode_to_sink(&mut sink), Ok(10));
    assert_eq!(decode(&sink), Ok(message));
}

#[test]
fn errors_display_their_fields() {
    let cases = [
        (BvllError::TooShort { length: 2 }, "2 octets"),
        (BvllError::NotBip { bvlc_type: 0x82 }, "0x82"),
        (
            BvllError::LengthMismatch {
                declared: 6,
                received: 8,
            },
            "length 6",
        ),
        (BvllError::UnknownFunction { function: 0x0C }, "0x0C"),
        (
            BvllError::PayloadLength {
                function: BvlcFunction::REGISTER_FOREIGN_DEVICE,
                length: 3,
            },
            "REGISTER_FOREIGN_DEVICE cannot carry a 3-octet payload",
        ),
        (
            BvllError::TableLength {
                length: 9,
                entry_length: 10,
            },
            "9 octets",
        ),
        (BvllError::AddressLength { length: 3 }, "got 3"),
        (BvllError::TooLong { length: 65536 }, "65536"),
        (BvllError::UndefinedResultCode { code: 0x70 }, "0x0070"),
        (
            BvllError::BufferExhausted {
                needed: 8,
                available: 7,
            },
            "need 8, have 7",
        ),
    ];
    for (error, text) in cases {
        let shown = error.to_string();
        assert!(shown.contains(text), "{shown:?} lacks {text:?}");
        let _: &dyn core::error::Error = &error;
    }
}
