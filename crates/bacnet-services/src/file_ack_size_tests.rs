use super::*;

#[test]
fn atomic_read_file_ack_size_wire_length_ceiling_without_payload_allocation() {
    assert_eq!(read_ack_octet_string_len(usize::MAX), None);
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(
            read_ack_octet_string_len(u32::MAX as usize),
            Some(u32::MAX as usize + 6)
        );
        assert_eq!(read_ack_octet_string_len(u32::MAX as usize + 1), None);
    }
    #[cfg(target_pointer_width = "32")]
    assert_eq!(read_ack_octet_string_len(u32::MAX as usize), None);
}

#[test]
fn atomic_read_file_ack_size_independent_wire_vectors() {
    for (start, signed) in [
        (-129, vec![0x32, 0xff, 0x7f]),
        (-128, vec![0x31, 0x80]),
        (0, vec![0x31, 0]),
        (127, vec![0x31, 0x7f]),
        (128, vec![0x32, 0, 0x80]),
        (32767, vec![0x32, 0x7f, 0xff]),
        (32768, vec![0x33, 0, 0x80, 0]),
        (8388607, vec![0x33, 0x7f, 0xff, 0xff]),
        (8388608, vec![0x34, 0, 0x80, 0, 0]),
        (i32::MIN, vec![0x34, 0x80, 0, 0, 0]),
        (i32::MAX, vec![0x34, 0x7f, 0xff, 0xff, 0xff]),
    ] {
        for (len, prefix) in [
            (0, vec![0x60]),
            (4, vec![0x64]),
            (5, vec![0x65, 5]),
            (253, vec![0x65, 253]),
            (254, vec![0x65, 254, 0, 254]),
            (65535, vec![0x65, 254, 255, 255]),
            (65536, vec![0x65, 255, 0, 1, 0, 0]),
        ] {
            for record in [false, true] {
                let data = vec![42; len];
                let ack = AtomicReadFileAck {
                    end_of_file: false,
                    access: if record {
                        FileReadAckMethod::Record {
                            file_start_record: start,
                            returned_record_count: 2,
                            file_record_data: vec![vec![], data.clone()],
                        }
                    } else {
                        FileReadAckMethod::Stream {
                            file_start_position: start,
                            file_data: data.clone(),
                        }
                    },
                };
                let mut expected = vec![0x10, if record { 0x1e } else { 0x0e }];
                expected.extend_from_slice(&signed);
                if record {
                    expected.extend_from_slice(&[0x21, 2, 0x60]);
                }
                expected.extend_from_slice(&prefix);
                expected.extend_from_slice(&data);
                expected.push(if record { 0x1f } else { 0x0f });
                assert_eq!(ack.encoded_len_bounded(expected.len() - 1), None);
                assert_eq!(
                    ack.encoded_len_bounded(expected.len()),
                    Some(expected.len())
                );
                assert_eq!(ack.encoded_len_bounded(usize::MAX), Some(expected.len()));
                let mut encoded = BytesMut::new();
                ack.encode(&mut encoded);
                assert_eq!(&encoded[..], &expected);
            }
        }
    }
}

#[test]
fn atomic_read_file_ack_size_record_count_widths_and_empty_results() {
    for (count, prefix) in [
        (0, vec![0x21, 0]),
        (255, vec![0x21, 255]),
        (256, vec![0x22, 1, 0]),
        (65535, vec![0x22, 255, 255]),
        (65536, vec![0x23, 1, 0, 0]),
    ] {
        let ack = AtomicReadFileAck {
            end_of_file: true,
            access: FileReadAckMethod::Record {
                file_start_record: 0,
                returned_record_count: count,
                file_record_data: vec![vec![]; count as usize],
            },
        };
        let mut expected = vec![0x11, 0x1e, 0x31, 0];
        expected.extend_from_slice(&prefix);
        expected.resize(expected.len() + count as usize, 0x60);
        expected.push(0x1f);
        assert_eq!(ack.encoded_len_bounded(expected.len() - 1), None);
        assert_eq!(
            ack.encoded_len_bounded(expected.len()),
            Some(expected.len())
        );
        let mut encoded = BytesMut::new();
        ack.encode(&mut encoded);
        assert_eq!(&encoded[..], &expected);
    }
}
