//! File services keep routed addressing, both access arms and typed ACK checks.
use super::*;
use crate::discovery::RoutedDeviceConfig;
use bacnet_encoding::apdu::{self, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileAck, AtomicWriteFileRequest,
    FileAccessMethod, FileReadAckMethod, FileWriteAccessMethod, FileWriteAckMethod,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, Segmentation};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

const INSTANCE: u32 = 1234;
const ROUTER: &[u8] = &[2];
const NETWORK: u16 = 100;
const STATION: &[u8] = &[5];

struct Peer {
    transport: LoopbackTransport,
    frames: mpsc::Receiver<ReceivedNpdu>,
    destinations: mpsc::UnboundedReceiver<MacAddr>,
    routed: bool,
}

async fn site(routed: bool) -> (BACnetClient<LoopbackTransport>, Peer) {
    let (mut local, mut remote) = LoopbackTransport::pair(&[1][..], ROUTER);
    let destinations = local.record_unicast_destinations();
    let frames = remote.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(local)
        .apdu_timeout_ms(1000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    if routed {
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: INSTANCE,
                router_mac: ROUTER.to_vec(),
                remote_network: NETWORK,
                remote_mac: STATION.to_vec(),
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::NONE,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
    } else {
        client.add_device(INSTANCE, ROUTER).await.unwrap();
    }
    (
        client,
        Peer {
            transport: remote,
            frames,
            destinations,
            routed,
        },
    )
}

impl Peer {
    async fn request(&mut self, service: ConfirmedServiceChoice) -> ConfirmedRequest {
        let frame = timeout(Duration::from_secs(5), self.frames.recv())
            .await
            .expect("request watchdog")
            .unwrap();
        assert!(!frame.link_layer_group);
        assert_eq!(self.destinations.recv().await.unwrap().as_slice(), ROUTER);
        let npdu = decode_npdu(frame.npdu).unwrap();
        if self.routed {
            let destination = npdu.destination.expect("file request lost DNET/DADR");
            assert_eq!(destination.network, NETWORK);
            assert_eq!(destination.mac_address.as_slice(), STATION);
        } else {
            assert!(npdu.destination.is_none());
        }
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected confirmed request")
        };
        assert_eq!(request.service_choice, service);
        request
    }
    async fn send(&self, apdu: Apdu) {
        let mut payload = BytesMut::new();
        apdu::encode_apdu(&mut payload, &apdu).unwrap();
        let mut bytes = BytesMut::new();
        encode_npdu(
            &mut bytes,
            &Npdu {
                source: self.routed.then(|| NpduAddress {
                    network: NETWORK,
                    mac_address: MacAddr::from_slice(STATION),
                }),
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.transport.send_unicast(&bytes, &[1]).await.unwrap();
    }
    async fn ack(&self, request: &ConfirmedRequest, bytes: Bytes) {
        self.send(Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: bytes,
        }))
        .await;
    }
    async fn error(&self, request: &ConfirmedRequest) {
        self.send(Apdu::Error(ErrorPdu {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::FILE_ACCESS_DENIED,
            error_data: Bytes::new(),
        }))
        .await;
    }
}

fn file() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::FILE, 17).unwrap()
}
fn stream() -> FileAccessMethod {
    FileAccessMethod::Stream {
        file_start_position: 7,
        requested_octet_count: 2,
    }
}
fn record() -> FileAccessMethod {
    FileAccessMethod::Record {
        file_start_record: 3,
        requested_record_count: 2,
    }
}
fn stream_ack() -> AtomicReadFileAck {
    AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 7,
            file_data: vec![0, 0xff],
        },
    }
}
fn record_ack() -> AtomicReadFileAck {
    AtomicReadFileAck {
        end_of_file: false,
        access: FileReadAckMethod::Record {
            file_start_record: 3,
            returned_record_count: 2,
            file_record_data: vec![vec![], vec![0, 0xff, 0x80]],
        },
    }
}
fn encoded(ack: &AtomicReadFileAck) -> Bytes {
    let mut bytes = BytesMut::new();
    ack.encode(&mut bytes);
    bytes.freeze()
}
fn assert_read(request: &ConfirmedRequest, access: &FileAccessMethod) {
    assert_eq!(
        AtomicReadFileRequest::decode(&request.service_request).unwrap(),
        AtomicReadFileRequest {
            file_identifier: file(),
            access: access.clone()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn device_file_reads_preserve_raw_and_decoded_stream_and_record_acks() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        for (access, expected) in [(stream(), stream_ack()), (record(), record_ack())] {
            let bytes = encoded(&expected);
            let (result, ()) = tokio::join!(
                client.atomic_read_file_from_device(INSTANCE, file(), access.clone()),
                async {
                    let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
                    assert_read(&request, &access);
                    peer.ack(&request, bytes.clone()).await;
                }
            );
            assert_eq!(result.unwrap(), bytes);
            let (result, ()) = tokio::join!(
                client.atomic_read_file_decoded_from_device(INSTANCE, file(), access.clone()),
                async {
                    let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
                    assert_read(&request, &access);
                    peer.ack(&request, bytes.clone()).await;
                }
            );
            assert_eq!(result.unwrap(), expected);
        }
        // The raw API intentionally preserves an undecodable service payload.
        let (result, ()) = tokio::join!(
            client.atomic_read_file_from_device(INSTANCE, file(), stream()),
            async {
                let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
                peer.ack(&request, Bytes::from_static(&[0xff])).await;
            }
        );
        assert_eq!(result.unwrap().as_ref(), &[0xff]);
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn routed_decoded_file_read_refuses_wrong_arm_oversized_and_malformed_acks() {
    let (mut client, mut peer) = site(true).await;
    let mut big_stream = stream_ack();
    let FileReadAckMethod::Stream { file_data, .. } = &mut big_stream.access else {
        unreachable!()
    };
    file_data.push(1);
    let mut big_record = record_ack();
    let FileReadAckMethod::Record {
        returned_record_count,
        file_record_data,
        ..
    } = &mut big_record.access
    else {
        unreachable!()
    };
    *returned_record_count = 3;
    file_record_data.push(vec![1]);
    let mut mismatched = record_ack();
    let FileReadAckMethod::Record {
        returned_record_count,
        ..
    } = &mut mismatched.access
    else {
        unreachable!()
    };
    *returned_record_count = 1;
    let mut trailing = encoded(&stream_ack()).to_vec();
    trailing.push(0);
    for (access, bytes) in [
        (stream(), encoded(&record_ack())),
        (record(), encoded(&stream_ack())),
        (stream(), encoded(&big_stream)),
        (record(), encoded(&big_record)),
        (record(), encoded(&mismatched)),
        (stream(), Bytes::from_static(&[0x11, 0x0e])),
        (stream(), Bytes::from(trailing)),
    ] {
        let (result, ()) = tokio::join!(
            client.atomic_read_file_decoded_from_device(INSTANCE, file(), access.clone()),
            async {
                let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
                assert_read(&request, &access);
                peer.ack(&request, bytes).await;
            }
        );
        assert!(
            matches!(
                result,
                Err(Error::Decoding(_) | Error::BufferTooShort { .. })
            ),
            "{result:?}"
        );
    }
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn device_file_writes_preserve_stream_append_and_record_request_and_raw_ack() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        for (access, ack) in [
            (
                FileWriteAccessMethod::Stream {
                    file_start_position: -1,
                    file_data: vec![0xff, 0, 0x80],
                },
                FileWriteAckMethod::Stream {
                    file_start_position: 9,
                },
            ),
            (
                FileWriteAccessMethod::Record {
                    file_start_record: 3,
                    record_count: 3,
                    file_record_data: vec![vec![], vec![0, 0xff], vec![0x7f]],
                },
                FileWriteAckMethod::Record {
                    file_start_record: 3,
                },
            ),
        ] {
            let mut bytes = BytesMut::new();
            AtomicWriteFileAck {
                access: ack.clone(),
            }
            .encode(&mut bytes);
            let bytes = bytes.freeze();
            let (result, ()) = tokio::join!(
                client.atomic_write_file_from_device(INSTANCE, file(), access.clone()),
                async {
                    let request = peer
                        .request(ConfirmedServiceChoice::ATOMIC_WRITE_FILE)
                        .await;
                    assert_eq!(
                        AtomicWriteFileRequest::decode(&request.service_request).unwrap(),
                        AtomicWriteFileRequest {
                            file_identifier: file(),
                            access: access.clone()
                        }
                    );
                    peer.ack(&request, bytes.clone()).await;
                }
            );
            let result = result.unwrap();
            assert_eq!(result, bytes);
            assert_eq!(AtomicWriteFileAck::decode(&result).unwrap().access, ack);
        }
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn routed_file_services_preserve_protocol_errors_and_missing_device_sends_nothing() {
    let (mut client, mut peer) = site(true).await;
    let write = FileWriteAccessMethod::Stream {
        file_start_position: 0,
        file_data: vec![1],
    };
    assert!(
        client
            .atomic_read_file_from_device(INSTANCE + 1, file(), stream())
            .await
            .is_err()
    );
    assert!(
        client
            .atomic_read_file_decoded_from_device(INSTANCE + 1, file(), record())
            .await
            .is_err()
    );
    assert!(
        client
            .atomic_write_file_from_device(INSTANCE + 1, file(), write.clone())
            .await
            .is_err()
    );
    assert!(peer.frames.try_recv().is_err());
    assert!(peer.destinations.try_recv().is_err());
    let (raw, ()) = tokio::join!(
        client.atomic_read_file_from_device(INSTANCE, file(), stream()),
        async {
            let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
            peer.error(&request).await;
        }
    );
    let (decoded, ()) = tokio::join!(
        client.atomic_read_file_decoded_from_device(INSTANCE, file(), record()),
        async {
            let request = peer.request(ConfirmedServiceChoice::ATOMIC_READ_FILE).await;
            peer.error(&request).await;
        }
    );
    let (written, ()) = tokio::join!(
        client.atomic_write_file_from_device(INSTANCE, file(), write),
        async {
            let request = peer
                .request(ConfirmedServiceChoice::ATOMIC_WRITE_FILE)
                .await;
            peer.error(&request).await;
        }
    );
    for error in [raw.unwrap_err(), decoded.unwrap_err(), written.unwrap_err()] {
        assert!(
            matches!(
                error,
                Error::Remote(RemoteError::Error {
                    class: ErrorClass::SERVICES,
                    code: ErrorCode::FILE_ACCESS_DENIED
                })
            ),
            "{error:?}"
        );
    }
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}
