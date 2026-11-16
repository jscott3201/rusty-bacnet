//! Each typed call's request and the result of its answer, pinned octet for
//! octet (#2588): the link MAC and NPDU it sends, and its answer as the
//! call returns it, for a station on this link and one behind a router.
//! The pins in `pins.rs` were recorded from the by-MAC and by-instance calls
//! before the typed calls moved onto one service description, and every way
//! of naming the station must still meet them.

use std::fmt::Debug;
use std::future::Future;

use bacnet_encoding::apdu::{ComplexAck, ErrorPdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;
use crate::discovery::RoutedDeviceConfig;

mod device;
mod events;
mod forms;
mod pins;
mod property;
mod remaining;
mod scope;
mod write_group;

pub(super) const CLIENT: &[u8] = &[0x01];
/// A device on this link.
pub(super) const PEER: &[u8] = &[0x02];
/// The router to `DNET`.
pub(super) const ROUTER: &[u8] = &[0x09];
pub(super) const DNET: u16 = 2;
/// The device behind `ROUTER`, on `DNET`.
pub(super) const DADR: &[u8] = &[0x05];
/// The instance of the device at `PEER`.
pub(super) const LOCAL: u32 = 78;
/// The instance of the device at `DNET:DADR`.
pub(super) const REMOTE: u32 = 77;
pub(super) const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A client on this link whose discovery table holds the device at `PEER`
/// and the one at `DNET:DADR` through `ROUTER`, and the frames it sends.
pub(super) struct Wire {
    pub(super) client: BACnetClient<TestTransport>,
    sent: SendLog,
    inbox: mpsc::Sender<ReceivedNpdu>,
}

pub(super) async fn wire() -> Wire {
    let (inbox, inbound) = mpsc::channel(16);
    let transport = TestTransport::builder()
        .local_mac(CLIENT)
        .broadcast_mac(&[0xFF])
        .inbound(inbound)
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 2_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    client.add_device(LOCAL, PEER).await.unwrap();
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: REMOTE,
            router_mac: ROUTER.to_vec(),
            remote_network: DNET,
            remote_mac: DADR.to_vec(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    Wire {
        client,
        sent,
        inbox,
    }
}

/// How the device answers a confirmed request.
#[derive(Clone)]
pub(super) enum Answer {
    Simple,
    /// A ComplexAck carrying these service octets.
    Complex(Vec<u8>),
    /// An Error PDU (object, unknown-object).
    Error,
}

/// The service octets `encode` writes.
pub(super) fn octets(encode: impl FnOnce(&mut BytesMut)) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode(&mut buf);
    buf.to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A sent frame as a pin writes it: the link MAC, then the NPDU.
fn frame_text(frame: &SentFrame) -> String {
    format!("{} {}", hex(&frame.mac), hex(&frame.npdu))
}

impl Wire {
    /// Run `call` while the device answers its one confirmed request with
    /// `answer`: the frame it sent and its result, as text.
    pub(super) async fn confirmed<R: Debug>(
        &self,
        call: impl Future<Output = Result<R, Error>>,
        answer: Answer,
    ) -> (String, String) {
        let at = self.sent.len();
        let (result, frame) = tokio::join!(call, self.answer(at, answer));
        (frame, format!("{result:?}"))
    }

    /// Run the unconfirmed `call`: the frames it sent (a Who-Is follows the
    /// client's first question for its network's number) and its result.
    pub(super) async fn unconfirmed(
        &self,
        call: impl Future<Output = Result<(), Error>>,
    ) -> (String, String) {
        let at = self.sent.len();
        let result = call.await;
        let frames: Vec<_> = self.sent.frames()[at..].iter().map(frame_text).collect();
        (frames.join(" | "), format!("{result:?}"))
    }

    /// Answer send `index` as the device does: from the frame's MAC, and
    /// through the router with the station as SNET/SADR when it was routed.
    async fn answer(&self, index: usize, answer: Answer) -> String {
        tokio::time::timeout(Duration::from_secs(5), self.sent.wait_for_len(index + 1))
            .await
            .expect("the call sends its request");
        let frame = self.sent.frame(index);
        let npdu = frame.decode_npdu();
        let Apdu::ConfirmedRequest(request) = frame.apdu() else {
            panic!("expected a confirmed request");
        };
        let (invoke_id, service_choice) = (request.invoke_id, request.service_choice);
        let reply = match answer {
            Answer::Simple => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            }),
            Answer::Complex(service_ack) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: Bytes::from(service_ack),
            }),
            Answer::Error => Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice,
                error_class: ErrorClass::OBJECT,
                error_code: ErrorCode::UNKNOWN_OBJECT,
                error_data: Bytes::new(),
            }),
        };
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &reply).unwrap();
        let back = Npdu {
            source: npdu.destination.map(|to| NpduAddress {
                network: to.network,
                mac_address: to.mac_address,
            }),
            payload: payload.freeze(),
            ..Npdu::default()
        };
        let mut buf = BytesMut::new();
        encode_npdu(&mut buf, &back).unwrap();
        let received =
            ReceivedNpdu::unverified(buf.freeze(), frame.mac.clone(), false, Vec::new(), None);
        self.inbox.send(received).await.unwrap();
        frame_text(&frame)
    }
}

/// `got` meets the pin named `case`. A case with no pin fails, printing what
/// it got in the table's form.
pub(super) fn check(case: &str, got: (String, String)) {
    let (frame, result) = got;
    match pins::PINS.iter().find(|(name, ..)| *name == case) {
        Some((_, pinned_frame, pinned_result)) => {
            assert_eq!(frame, *pinned_frame, "{case}: the frame sent");
            assert_eq!(result, *pinned_result, "{case}: the result");
        }
        None => panic!("no pin: (\"{case}\", \"{frame}\", r#\"{result}\"#),"),
    }
}

/// The exchange `make` builds, on the heap. `make` runs in this function's
/// frame, so a test of dozens of exchanges keeps one pointer per exchange
/// in its own: inline, a debug build gives each exchange's temporaries a
/// stack slot of their own, about 25 KiB each, and a test of 45 outgrew a
/// 1 MiB stack.
pub(super) fn boxed<'a, F: Future<Output = ()> + 'a>(
    make: impl FnOnce() -> F,
) -> std::pin::Pin<Box<dyn Future<Output = ()> + 'a>> {
    Box::pin(make())
}

/// Pin one exchange: `$call` with the wire's client as `$c`, answered with
/// `$answer` (or unconfirmed), against the pin named `$case`. Each one is a
/// future of its own ([`boxed`]).
macro_rules! pin {
    ($case:expr, $answer:expr, |$c:ident| $call:expr) => {
        $crate::client::typed_wire_tests::boxed(|| async {
            let wire = $crate::client::typed_wire_tests::wire().await;
            let $c = &wire.client;
            $crate::client::typed_wire_tests::check($case, wire.confirmed($call, $answer).await);
        })
        .await
    };
    ($case:expr, |$c:ident| $call:expr) => {
        $crate::client::typed_wire_tests::boxed(|| async {
            let wire = $crate::client::typed_wire_tests::wire().await;
            let $c = &wire.client;
            $crate::client::typed_wire_tests::check($case, wire.unconfirmed($call).await);
        })
        .await
    };
}
pub(super) use pin;
