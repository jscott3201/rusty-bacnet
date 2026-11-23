//! An acknowledgement of the other kind than the request's service takes
//! (#2721), on a standalone client and the shared test transport. A
//! Complex-ACK for WriteProperty and a Simple-ACK for ReadProperty each end
//! the request at once with [`Error::UnexpectedAck`], never as a success or
//! a timeout: the transaction closes, no retry goes and nothing is sent to
//! the device. A service the standard doesn't define takes either, and so
//! does ConfirmedPrivateTransfer, whose typed call reports a Simple-ACK
//! itself; an acknowledgement labelled for another service is still
//! ignored (#341). A client set to accept a Complex-ACK for a Simple-ACK
//! service takes one as success; a Simple-ACK where data is due stays an
//! error.

use std::sync::Arc;

use bacnet_encoding::apdu::{self, Apdu, ComplexAck, SimpleAck};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SentFrame, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use super::routed_path_limit_tests::inject_apdu;
use super::{BACnetClient, ClientConfig};

const PEER: [u8; 1] = [5];

struct Rig {
    client: Arc<BACnetClient<TestTransport>>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    frames: mpsc::UnboundedReceiver<SentFrame>,
}

impl Rig {
    async fn new() -> Self {
        Self::with(ClientConfig::default()).await
    }

    async fn with(config: ClientConfig) -> Self {
        let (inbound, inbound_rx) = mpsc::channel(16);
        let (frames_tx, frames) = mpsc::unbounded_channel();
        let transport = TestTransport::builder()
            .local_mac(&[1])
            .inbound(inbound_rx)
            .on_send(move |frame| {
                let _ = frames_tx.send(frame);
                async { Ok(()) }
            })
            .build();
        let client = BACnetClient::start(config, transport).await.unwrap();
        Self {
            client: Arc::new(client),
            inbound,
            frames,
        }
    }

    /// The invoke ID and service of the next request the client sends.
    async fn next(&mut self) -> (u8, ConfirmedServiceChoice) {
        let frame = timeout(Duration::from_secs(60), self.frames.recv())
            .await
            .expect("no request")
            .unwrap();
        let npdu = decode_npdu(frame.npdu).unwrap();
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected a ConfirmedRequest")
        };
        (request.invoke_id, request.service_choice)
    }

    async fn answer(&self, apdu: Apdu) {
        inject_apdu(&self.inbound, &PEER, None, apdu).await;
    }

    /// No transaction is left open, and nothing more goes out, even after
    /// every retry timer would have run.
    async fn closed_and_quiet(&mut self) {
        assert_eq!(self.client.tsm.lock().pending_count(), 0);
        assert_eq!(self.client.tsm.lock().coordinated_active_count(), 0);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(self.frames.try_recv().is_err(), "something more was sent");
    }

    async fn stop(self) {
        let mut client = Arc::try_unwrap(self.client).ok().unwrap();
        client.stop().await.unwrap();
    }
}

fn simple(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    })
}

fn complex(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_ack: Bytes::from_static(&[0x21, 0x2A]),
    })
}

fn ai0() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 0).unwrap()
}

/// A Complex-ACK naming WriteProperty fails the write at once: not a
/// success, and not a timeout either.
#[tokio::test(start_paused = true)]
async fn a_complex_ack_for_a_write_fails_it() {
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let write = tokio::spawn(async move {
        client
            .write_property(
                &PEER[..],
                ai0(),
                PropertyIdentifier::DESCRIPTION,
                None,
                vec![0x75, 0x02, 0x00, 0x41],
                None,
            )
            .await
    });
    let (invoke_id, service) = rig.next().await;
    assert_eq!(service, ConfirmedServiceChoice::WRITE_PROPERTY);
    rig.answer(complex(invoke_id, service)).await;
    let result = write.await.unwrap();
    assert!(
        matches!(
            result,
            Err(Error::UnexpectedAck {
                service: ConfirmedServiceChoice::WRITE_PROPERTY,
                complex: true
            })
        ),
        "{result:?}"
    );
    rig.closed_and_quiet().await;
    rig.stop().await;
}

/// The reverse: a Simple-ACK naming ReadProperty, whose answer carries the
/// value, fails the read with the same error, not a decoding error.
#[tokio::test(start_paused = true)]
async fn a_simple_ack_for_a_read_fails_it() {
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let read = tokio::spawn(async move {
        client
            .read_property(&PEER[..], ai0(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    });
    let (invoke_id, service) = rig.next().await;
    assert_eq!(service, ConfirmedServiceChoice::READ_PROPERTY);
    rig.answer(simple(invoke_id, service)).await;
    let result = read.await.unwrap();
    assert!(
        matches!(
            result,
            Err(Error::UnexpectedAck {
                service: ConfirmedServiceChoice::READ_PROPERTY,
                complex: false
            })
        ),
        "{result:?}"
    );
    rig.closed_and_quiet().await;
    rig.stop().await;
}

/// Every service in each kind, through the raw request: the right kind
/// completes, the wrong one fails. A service the standard doesn't define
/// (40) or removed (13) takes either kind, as before, and so does
/// ConfirmedPrivateTransfer (18).
#[tokio::test(start_paused = true)]
async fn each_service_takes_its_own_kind_and_undefined_ones_take_either() {
    let mut rig = Rig::new().await;
    // ConfirmedPrivateTransfer (18) takes either, and is in neither.
    let complex_services = [3, 4, 6, 7, 10, 12, 14, 21, 23, 26, 29, 33];
    let simple_services = [
        0, 1, 2, 5, 8, 9, 11, 15, 16, 17, 19, 20, 22, 27, 28, 30, 31, 32,
    ];
    for raw in (0..=34).chain([40]) {
        let service = ConfirmedServiceChoice::from_raw(raw);
        for complex_ack in [false, true] {
            let client = Arc::clone(&rig.client);
            let request =
                tokio::spawn(
                    async move { client.confirmed_request(&PEER[..], service, &[]).await },
                );
            let (invoke_id, _) = rig.next().await;
            let answer = if complex_ack {
                complex(invoke_id, service)
            } else {
                simple(invoke_id, service)
            };
            rig.answer(answer).await;
            let result = request.await.unwrap();
            let wrong = if complex_ack {
                simple_services.contains(&raw)
            } else {
                complex_services.contains(&raw)
            };
            if wrong {
                assert!(
                    matches!(&result, Err(Error::UnexpectedAck { service: s, complex: c })
                        if *s == service && *c == complex_ack),
                    "service {raw}, complex {complex_ack}: {result:?}"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "service {raw}, complex {complex_ack}: {result:?}"
                );
            }
        }
    }
    rig.closed_and_quiet().await;
    rig.stop().await;
}

/// A Complex-ACK labelled for another service is no answer to the
/// request, as before (#341): the request waits on, and its own Simple-ACK
/// completes it.
#[tokio::test(start_paused = true)]
async fn an_ack_labelled_for_another_service_is_still_ignored() {
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let request = tokio::spawn(async move {
        client
            .confirmed_request(&PEER[..], ConfirmedServiceChoice::WRITE_PROPERTY, &[])
            .await
    });
    let (invoke_id, service) = rig.next().await;
    rig.answer(complex(invoke_id, ConfirmedServiceChoice::READ_PROPERTY))
        .await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!request.is_finished());
    rig.answer(simple(invoke_id, service)).await;
    assert!(request.await.unwrap().unwrap().is_empty());
    rig.stop().await;
}

/// Its text names the service and both kinds; it is no peer's Error,
/// Reject or Abort.
#[test]
fn the_error_is_no_remote_answer() {
    let error = Error::UnexpectedAck {
        service: ConfirmedServiceChoice::WRITE_PROPERTY,
        complex: true,
    };
    assert_eq!(error.remote(), None::<RemoteError>);
    assert!(
        error
            .to_string()
            .contains("write-property with a Complex-ACK")
    );
}

fn lenient() -> ClientConfig {
    ClientConfig {
        accept_complex_ack_for_simple_services: true,
        ..ClientConfig::default()
    }
}

/// A client set to accept a Complex-ACK for a Simple-ACK service, for a
/// device in the field that answers a write so, takes it as the write's
/// success; a Simple-ACK for a read, where data is due, still fails.
#[tokio::test(start_paused = true)]
async fn the_option_takes_a_complex_ack_for_a_write_and_never_a_simple_one_for_a_read() {
    let mut rig = Rig::with(lenient()).await;
    let client = Arc::clone(&rig.client);
    let write = tokio::spawn(async move {
        client
            .write_property(
                &PEER[..],
                ai0(),
                PropertyIdentifier::DESCRIPTION,
                None,
                vec![0x75, 0x02, 0x00, 0x41],
                None,
            )
            .await
    });
    let (invoke_id, service) = rig.next().await;
    rig.answer(complex(invoke_id, service)).await;
    write.await.unwrap().unwrap();

    let client = Arc::clone(&rig.client);
    let read = tokio::spawn(async move {
        client
            .read_property(&PEER[..], ai0(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    });
    let (invoke_id, service) = rig.next().await;
    rig.answer(simple(invoke_id, service)).await;
    let result = read.await.unwrap();
    assert!(
        matches!(result, Err(Error::UnexpectedAck { complex: false, .. })),
        "{result:?}"
    );
    rig.closed_and_quiet().await;
    rig.stop().await;
}

/// ConfirmedPrivateTransfer: the raw request takes either kind, as vendor
/// devices answer it both ways; the typed call, which needs the
/// acknowledgement's vendor and service numbers, reports a Simple-ACK as
/// the wrong kind.
#[tokio::test(start_paused = true)]
async fn a_private_transfer_takes_either_raw_and_the_typed_call_needs_its_numbers() {
    use bacnet_services::private_transfer::PrivateTransferRequest;
    let transfer = ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER;
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let raw = tokio::spawn(async move { client.confirmed_request(&PEER[..], transfer, &[]).await });
    let (invoke_id, _) = rig.next().await;
    rig.answer(simple(invoke_id, transfer)).await;
    assert!(raw.await.unwrap().unwrap().is_empty());

    let client = Arc::clone(&rig.client);
    let typed = tokio::spawn(async move {
        let request = PrivateTransferRequest {
            vendor_id: 555,
            service_number: 7,
            service_parameters: None,
        };
        client.confirmed_private_transfer(&PEER[..], &request).await
    });
    let (invoke_id, _) = rig.next().await;
    rig.answer(simple(invoke_id, transfer)).await;
    let result = typed.await.unwrap();
    assert!(
        matches!(
            result,
            Err(Error::UnexpectedAck { service, complex: false }) if service == transfer
        ),
        "{result:?}"
    );
    rig.closed_and_quiet().await;
    rig.stop().await;
}

/// A Complex-ACK for a write that arrives in segments is acknowledged
/// segment by segment and caught when it is whole: the write fails with
/// the wrong kind, and nothing more is sent. A client that accepts a
/// Complex-ACK for a write takes the reassembled one as success.
#[tokio::test(start_paused = true)]
async fn a_segmented_complex_ack_for_a_write_is_caught_when_whole() {
    for accept in [false, true] {
        let config = ClientConfig {
            accept_complex_ack_for_simple_services: accept,
            ..ClientConfig::default()
        };
        let mut rig = Rig::with(config).await;
        let client = Arc::clone(&rig.client);
        let write = tokio::spawn(async move {
            client
                .confirmed_request(&PEER[..], ConfirmedServiceChoice::WRITE_PROPERTY, &[])
                .await
        });
        let (invoke_id, service) = rig.next().await;
        for (sequence, more_follows) in [(0, true), (1, false)] {
            let segment = Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows,
                invoke_id,
                sequence_number: Some(sequence),
                proposed_window_size: Some(1),
                service_choice: service,
                service_ack: Bytes::from_static(&[0x21, 0x2A]),
            });
            rig.answer(segment).await;
            let frame = timeout(Duration::from_secs(60), rig.frames.recv())
                .await
                .expect("a SegmentACK")
                .unwrap();
            assert!(
                matches!(frame.apdu(), Apdu::SegmentAck(_)),
                "segment {sequence}"
            );
        }
        let result = write.await.unwrap();
        if accept {
            assert_eq!(result.unwrap().as_ref(), [0x21, 0x2A, 0x21, 0x2A]);
        } else {
            assert!(
                matches!(result, Err(Error::UnexpectedAck { complex: true, .. })),
                "{result:?}"
            );
        }
        rig.closed_and_quiet().await;
        rig.stop().await;
    }
}
