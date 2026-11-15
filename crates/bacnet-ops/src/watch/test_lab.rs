//! The hub's test lab: an in-process device over a loopback link, behind a
//! client link that records what it sends and can play a misbehaving
//! device (silent, refusing RPM, failing batches, swallowing subscriptions).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_encoding::constructed::decode_cov_subscription;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{
    AbortReason, EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
    RejectReason,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::Bytes;
use tokio::time::Instant;

use super::hub::{Hub, WatchOptions};
use super::{Point, Watched};
use crate::client::Client;
use crate::limits::RequestLimits;

pub(super) const LAB: u32 = 1001;

/// Where the lab device answers.
pub(super) fn lab_at() -> crate::DeviceAddress {
    crate::DeviceAddress::local(&[2]).unwrap()
}
/// A station on network 9 behind the lab device's MAC, as a router, which
/// the lab device doesn't answer for.
pub(super) fn routed_at() -> crate::DeviceAddress {
    crate::DeviceAddress::new(9, &[7]).unwrap()
}

pub(super) const RPM: u8 = 14;
pub(super) const RP: u8 = 12;
pub(super) const SUBSCRIBE_COV: u8 = 5;
/// SubscribeCOVProperty.
pub(super) const SUBSCRIBE_COV_PROPERTY: u8 = 28;
/// ReadProperty.
pub(super) const READ_PROPERTY: u8 = 12;

/// How the client's link misbehaves.
#[derive(Default)]
pub(super) struct Quirks {
    /// Nothing sent arrives.
    pub(super) muted: AtomicBool,
    /// The link refuses to send a ReadProperty or an RPM: nothing leaves,
    /// and the send fails, as a router's application refuses a station
    /// that names no network.
    pub(super) unsent_reads: AtomicBool,
    /// The link refuses to send a SubscribeCOV or SubscribeCOVProperty, the
    /// same way: a link reconnecting as a watch starts.
    pub(super) unsent_cov: AtomicBool,
    /// Every RPM is rejected as an unknown service.
    pub(super) refuse_rpm: AtomicBool,
    /// Every RPM fails with an OBJECT-class error, as a device failing a
    /// whole request for one bad object.
    pub(super) fail_rpm: AtomicBool,
    /// The next RPMs abort as too large to send.
    pub(super) abort_rpm: AtomicU8,
    /// The next SubscribeCOV requests vanish, retries included.
    pub(super) swallow_cov: AtomicU8,
    /// Every SubscribeCOV and SubscribeCOVProperty is rejected as an unknown
    /// service, as by a device without COV.
    pub(super) refuse_cov: AtomicBool,
    /// The next SubscribeCOV requests are refused for a full subscription
    /// table.
    pub(super) full_cov: AtomicU8,
    /// The device's answers to RPM take half a second.
    pub(super) slow_rpm_ack: AtomicBool,
    /// The device's acknowledgements of SubscribeCOV take half a second.
    pub(super) slow_cov_ack: AtomicBool,
    /// The device's acknowledgements of SubscribeCOV come half a second
    /// late, after what it sent meanwhile: its notifications overtake them.
    pub(super) late_cov_ack: AtomicBool,
}

/// Whether `npdu` acknowledges a SubscribeCOV.
fn is_cov_ack(npdu: &ReceivedNpdu) -> bool {
    is_ack_of(npdu, 2, SUBSCRIBE_COV)
}

/// Whether `npdu` is an answer of PDU type `kind` (2 simple, 3 complex) to
/// `service`.
fn is_ack_of(npdu: &ReceivedNpdu, kind: u8, service: u8) -> bool {
    decode_npdu(npdu.npdu.clone()).is_ok_and(|decoded| {
        let apdu = &decoded.payload;
        apdu.len() >= 3 && apdu[0] >> 4 == kind && apdu[2] == service
    })
}

fn take(counter: &AtomicU8) -> bool {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
}

/// The client's link to the lab device: `inner`, relayed through a channel
/// the link can also inject answers into, recording each confirmed request
/// in `sent` and playing `quirks`.
pub(super) fn link(
    inner: LoopbackTransport,
    quirks: Arc<Quirks>,
    sent: Arc<Mutex<Vec<(u8, Instant)>>>,
) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let (late, injecting) = (builder.handle(), builder.handle());
    let inbound_quirks = Arc::clone(&quirks);
    let send_quirks = Arc::clone(&quirks);
    builder
        .on_send(move |frame| {
            let reads = send_quirks.unsent_reads.load(Ordering::Relaxed);
            let cov = send_quirks.unsent_cov.load(Ordering::Relaxed);
            let read = !frame.broadcast
                && decode_npdu(frame.npdu.clone()).is_ok_and(|decoded| {
                    let apdu = &decoded.payload;
                    let confirmed = apdu.len() >= 4 && apdu[0] >> 4 == 0;
                    confirmed
                        && ((reads && [READ_PROPERTY, RPM].contains(&apdu[3]))
                            || (cov && [SUBSCRIBE_COV, SUBSCRIBE_COV_PROPERTY].contains(&apdu[3])))
                });
            async move {
                match read {
                    true => Err(bacnet_types::error::Error::Encoding(
                        "the link refused the send".into(),
                    )),
                    false => Ok(()),
                }
            }
        })
        .on_inbound(move |npdu| {
            let quirks = Arc::clone(&inbound_quirks);
            let late = late.clone();
            async move {
                if quirks.slow_cov_ack.load(Ordering::Relaxed) && is_cov_ack(&npdu) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if quirks.slow_rpm_ack.load(Ordering::Relaxed) && is_ack_of(&npdu, 3, RPM) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if quirks.late_cov_ack.load(Ordering::Relaxed) && is_cov_ack(&npdu) {
                    if let Some(sender) = late.inbound() {
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let _ = sender.send(npdu).await;
                        });
                    }
                    return None;
                }
                Some(npdu)
            }
        })
        .intercept(move |frame| {
            let verdict = if frame.broadcast {
                true
            } else {
                forwards(&frame, &quirks, &sent, &injecting)
            };
            async move { verdict }
        })
        .build()
}

/// Answer request `invoke` with `apdu`'s tail, as the device would.
fn answer(handle: &TestTransportHandle, apdu: Vec<u8>) {
    let mut frame = vec![1, 0];
    frame.extend(apdu);
    inject(handle, frame);
}

/// Answer a request to [`routed_at`] with `apdu`, from that station
/// through the lab device's MAC.
fn answer_routed(handle: &TestTransportHandle, apdu: Vec<u8>) {
    // SNET 9, SADR 7.
    let mut frame = vec![1, 0x08, 0, 9, 1, 7];
    frame.extend(apdu);
    inject(handle, frame);
}

fn inject(handle: &TestTransportHandle, frame: Vec<u8>) {
    if let Some(inject) = handle.inbound() {
        let _ = inject.try_send(ReceivedNpdu::unverified(
            Bytes::from(frame),
            MacAddr::from_slice(&[2]),
            false,
            Vec::new(),
            None,
        ));
    }
}

/// Whether a unicast goes on to the device, after the quirks have had their
/// say: a confirmed request is recorded, and the ones the device would
/// refuse or abort are answered here instead.
fn forwards(
    frame: &bacnet_transport::test_transport::SentFrame,
    quirks: &Quirks,
    sent: &Mutex<Vec<(u8, Instant)>>,
    handle: &TestTransportHandle,
) -> bool {
    let Ok(decoded) = decode_npdu(frame.npdu.clone()) else {
        return true;
    };
    let apdu = &decoded.payload;
    if !decoded.is_network_message() && apdu.len() >= 4 && apdu[0] >> 4 == 0 {
        let (invoke, service) = (apdu[2], apdu[3]);
        sent.lock().unwrap().push((service, Instant::now()));
        if quirks.muted.load(Ordering::Relaxed) {
            return false;
        }
        if decoded.destination.is_some() {
            // The lab device routes nothing. For the station behind it, an
            // RPM gets AI 1's Present_Value (10.0) as if from the station,
            // any other read the station's refusal of the object, and a
            // subscription no answer.
            match service {
                RPM => answer_routed(
                    handle,
                    vec![
                        0x30, invoke, RPM, 0x0C, 0, 0, 0, 1, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x41,
                        0x20, 0, 0, 0x4F, 0x1F,
                    ],
                ),
                SUBSCRIBE_COV => {}
                _ => {
                    let class = ErrorClass::OBJECT.to_raw() as u8;
                    let code = ErrorCode::UNKNOWN_OBJECT.to_raw() as u8;
                    answer_routed(handle, vec![0x50, invoke, service, 0x91, class, 0x91, code]);
                }
            }
            return false;
        }
        if service == RPM {
            if quirks.refuse_rpm.load(Ordering::Relaxed) {
                let reason = RejectReason::UNRECOGNIZED_SERVICE.to_raw();
                answer(handle, vec![0x60, invoke, reason]);
                return false;
            }
            if take(&quirks.abort_rpm) {
                let reason = AbortReason::SEGMENTATION_NOT_SUPPORTED.to_raw();
                answer(handle, vec![0x71, invoke, reason]);
                return false;
            }
            if quirks.fail_rpm.load(Ordering::Relaxed) {
                let class = ErrorClass::OBJECT.to_raw() as u8;
                let code = ErrorCode::UNKNOWN_OBJECT.to_raw() as u8;
                answer(handle, vec![0x50, invoke, RPM, 0x91, class, 0x91, code]);
                return false;
            }
        }
        if service == SUBSCRIBE_COV && take(&quirks.swallow_cov) {
            return false;
        }
        if service == SUBSCRIBE_COV && take(&quirks.full_cov) {
            let class = ErrorClass::RESOURCES.to_raw() as u8;
            let code = ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u8;
            answer(
                handle,
                vec![0x50, invoke, SUBSCRIBE_COV, 0x91, class, 0x91, code],
            );
            return false;
        }
        if [SUBSCRIBE_COV, SUBSCRIBE_COV_PROPERTY].contains(&service)
            && quirks.refuse_cov.load(Ordering::Relaxed)
        {
            let reason = RejectReason::UNRECOGNIZED_SERVICE.to_raw();
            answer(handle, vec![0x60, invoke, reason]);
            return false;
        }
        true
    } else {
        !quirks.muted.load(Ordering::Relaxed)
    }
}

pub(super) struct Lab {
    pub(super) hub: Hub<TestTransport>,
    pub(super) client: Client<TestTransport>,
    pub(super) server: BACnetServer<LoopbackTransport>,
    pub(super) quirks: Arc<Quirks>,
    sent: Arc<Mutex<Vec<(u8, Instant)>>>,
}

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(super) fn ai(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, instance)
}

/// A notification class, which takes no COV subscription.
pub(super) fn nc(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::NOTIFICATION_CLASS, instance)
}

pub(super) fn object(object: ObjectIdentifier) -> Point {
    Point {
        device: lab_at(),
        object,
        watched: Watched::Object,
    }
}

impl Lab {
    /// The lab device holds AI 1-3 (10.0 %), BV 1, and notification
    /// classes 0-3. The client answers in 1 s with no retries.
    pub(super) async fn start(options: WatchOptions) -> Self {
        let identity = DeviceIdentity::new(LAB, 42).unwrap();
        let mut objects: Vec<Box<dyn BACnetObject>> = Vec::new();
        for instance in 1..=3 {
            let mut point = AnalogInputObject::new(
                instance,
                format!("AI {instance}"),
                EngineeringUnits::PERCENT,
            )
            .unwrap();
            point.set_present_value(10.0);
            objects.push(Box::new(point));
        }
        objects.push(Box::new(BinaryValueObject::new(1, "BV 1").unwrap()));
        for instance in 0..=3 {
            objects.push(Box::new(
                NotificationClass::new(instance, format!("NC {instance}")).unwrap(),
            ));
        }
        let database = build_database_with_extra(&identity, objects).unwrap();
        let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
        let server = BACnetServer::generic_builder()
            .device(identity)
            .database(database)
            .transport(server_port)
            .build()
            .await
            .unwrap();
        let quirks = Arc::new(Quirks::default());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let config = ClientConfig {
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            ..ClientConfig::default()
        };
        let link = link(client_port, Arc::clone(&quirks), Arc::clone(&sent));
        let low = BACnetClient::start(config, link).await.unwrap();
        low.add_device(LAB, &[2]).await.unwrap();
        let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
        let hub = Hub::new(client.clone(), options).unwrap();
        Self {
            hub,
            client,
            server,
            quirks,
            sent,
        }
    }

    /// Give the client's discovery table a router to [`routed_at`]'s
    /// network: the lab device's MAC.
    pub(super) async fn route(&self) {
        let config = bacnet_client::discovery::RoutedDeviceConfig {
            instance: 2002,
            router_mac: vec![2],
            remote_network: 9,
            remote_mac: vec![7],
            max_apdu_length: 1476,
            segmentation_supported: bacnet_types::enums::Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        };
        self.client
            .low_level()
            .add_routed_device(config)
            .await
            .unwrap();
    }

    /// Confirmed requests of `service` sent so far, at or after `from`.
    pub(super) fn sent(&self, service: u8, from: Instant) -> usize {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, at)| *s == service && *at >= from)
            .count()
    }

    /// When each confirmed request of `service` went.
    pub(super) fn sent_at(&self, service: u8) -> Vec<Instant> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| *s == service)
            .map(|(_, at)| *at)
            .collect()
    }

    /// Every confirmed request's service, in order.
    pub(super) fn services(&self) -> Vec<u8> {
        self.sent.lock().unwrap().iter().map(|(s, _)| *s).collect()
    }

    /// The device's subscription table.
    pub(super) async fn table(&self) -> Vec<BACnetCOVSubscription> {
        let value = self
            .server
            .read_local(
                &oid(ObjectType::DEVICE, LAB),
                PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
                None,
            )
            .await
            .unwrap();
        let PropertyValue::ApplicationData(data) = value else {
            panic!("an encoded list: {value:?}");
        };
        let mut entries = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let (entry, next) = decode_cov_subscription(&data, at).unwrap();
            entries.push(entry);
            at = next;
        }
        entries
    }

    pub(super) async fn stop(self) {
        assert!(self.hub.shutdown(Duration::from_secs(30)).await);
        drop(self.hub);
        let Some(low) = self.client.into_low_level() else {
            panic!("the hub let go of the client");
        };
        let Ok(mut low) = Arc::try_unwrap(low) else {
            panic!("the client holds the only low-level client");
        };
        low.stop().await.unwrap();
        let mut server = self.server;
        server.stop().await.unwrap();
    }
}

/// Let the lab's tasks run: paused time moves on only when all are idle.
pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}
