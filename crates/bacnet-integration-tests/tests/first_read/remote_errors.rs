//! A controlled UDP peer sends actual error and malformed-value responses.

use super::*;
use bacnet_encoding::apdu::{ComplexAck, ErrorPdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;

#[tokio::test]
async fn remote_property_error_and_malformed_value_exit_unsuccessfully() {
    for malformed in [false, true] {
        let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = socket.local_addr().unwrap();
        let child = Process::spawn(&process::binary(), &[&address.to_string()]).unwrap();
        let mut wire = [0; 2048];
        let (length, source) = timeout(WAIT, socket.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        let Bvll::OriginalUnicastNpdu(npdu) = bacnet_bip_codec::decode(&wire[..length]).unwrap()
        else {
            panic!("directed request required");
        };
        let npdu = decode_npdu(Bytes::copy_from_slice(npdu)).unwrap();
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("confirmed request required");
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::READ_PROPERTY
        );
        let read = ReadPropertyRequest::decode(&request.service_request).unwrap();
        assert_eq!(
            read.object_identifier,
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
        );
        assert_eq!(read.property_identifier, PropertyIdentifier::PRESENT_VALUE);
        assert_eq!(read.property_array_index, None);
        let response = if malformed {
            let ack = ReadPropertyACK {
                object_identifier: read.object_identifier,
                property_identifier: read.property_identifier,
                property_array_index: None,
                // Real application tag, but only one of its four value octets.
                property_value: vec![0x44, 0x42],
            };
            let mut service = BytesMut::new();
            ack.encode(&mut service);
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: service.freeze(),
            })
        } else {
            Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
                error_data: Bytes::new(),
            })
        };
        let mut apdu = BytesMut::new();
        encode_apdu(&mut apdu, &response).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: apdu.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        let response = Bvll::OriginalUnicastNpdu(&npdu);
        let mut wire = vec![0; response.encoded_len().unwrap()];
        response.encode_into(&mut wire).unwrap();
        socket.send_to(&wire, source).await.unwrap();
        let output = child.finish().await;
        drop(socket);
        assert_rebind(address);
        assert_rebind(source);
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = if malformed {
            "BufferTooShort"
        } else {
            "UNKNOWN_PROPERTY"
        };
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.to_lowercase().contains("timeout"), "{stderr}");
    }
}
