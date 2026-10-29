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
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{
    AbortReason, EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
    RejectReason,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{Hub, Point, WatchOptions, Watched};
use crate::engine::Engine;
use crate::limits::RequestLimits;

pub(super) const LAB: u32 = 1001;
pub(super) const RPM: u8 = 14;
pub(super) const RP: u8 = 12;
pub(super) const SUBSCRIBE_COV: u8 = 5;

/// How the client's link misbehaves.
#[derive(Default)]
pub(super) struct Quirks {
    /// Nothing sent arrives.
    pub(super) muted: AtomicBool,
    /// Every RPM is rejected as an unknown service.
    pub(super) refuse_rpm: AtomicBool,
    /// Every RPM fails with an OBJECT-class error, as a device failing a
    /// whole request for one bad object.
    pub(super) fail_rpm: AtomicBool,
    /// The next RPMs abort as too large to send.
    pub(super) abort_rpm: AtomicU8,
    /// The next SubscribeCOV requests vanish, retries included.
    pub(super) swallow_cov: AtomicU8,
    /// The device's acknowledgements of SubscribeCOV take half a second.
    pub(super) slow_cov_ack: AtomicBool,
}

/// The client's link to the lab device.
pub(super) struct Link {
    inner: LoopbackTransport,
    quirks: Arc<Quirks>,
    /// Each confirmed request sent: its service and when.
    sent: Arc<Mutex<Vec<(u8, Instant)>>>,
    inject: Arc<Mutex<Option<mpsc::Sender<ReceivedNpdu>>>>,
}

impl Link {
    /// Answer request `invoke` with `apdu`'s tail, as the device would.
    fn answer(&self, apdu: Vec<u8>) {
        let mut frame = vec![1, 0];
        frame.extend(apdu);
        let inject = self.inject.lock().unwrap().clone();
        if let Some(inject) = inject {
            let _ = inject.try_send(ReceivedNpdu::unverified(
                Bytes::from(frame),
                MacAddr::from_slice(&[2]),
                false,
                Vec::new(),
                None,
            ));
        }
    }
}

/// Whether `npdu` acknowledges a SubscribeCOV.
fn is_cov_ack(npdu: &ReceivedNpdu) -> bool {
    decode_npdu(npdu.npdu.clone()).is_ok_and(|decoded| {
        let apdu = &decoded.payload;
        apdu.len() >= 3 && apdu[0] >> 4 == 2 && apdu[2] == SUBSCRIBE_COV
    })
}

fn take(counter: &AtomicU8) -> bool {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
}

impl TransportPort for Link {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let mut from_device = self.inner.start().await?;
        let (sender, receiver) = mpsc::channel(256);
        *self.inject.lock().unwrap() = Some(sender.clone());
        let quirks = Arc::clone(&self.quirks);
        tokio::spawn(async move {
            while let Some(npdu) = from_device.recv().await {
                if quirks.slow_cov_ack.load(Ordering::Relaxed) && is_cov_ack(&npdu) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if sender.send(npdu).await.is_err() {
                    break;
                }
            }
        });
        Ok(receiver)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inject.lock().unwrap().take();
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let decoded = decode_npdu(Bytes::copy_from_slice(npdu))?;
        let apdu = &decoded.payload;
        if !decoded.is_network_message() && apdu.len() >= 4 && apdu[0] >> 4 == 0 {
            let (invoke, service) = (apdu[2], apdu[3]);
            self.sent.lock().unwrap().push((service, Instant::now()));
            let quirks = &self.quirks;
            if quirks.muted.load(Ordering::Relaxed) {
                return Ok(());
            }
            if service == RPM {
                if quirks.refuse_rpm.load(Ordering::Relaxed) {
                    let reason = RejectReason::UNRECOGNIZED_SERVICE.to_raw();
                    self.answer(vec![0x60, invoke, reason]);
                    return Ok(());
                }
                if take(&quirks.abort_rpm) {
                    let reason = AbortReason::SEGMENTATION_NOT_SUPPORTED.to_raw();
                    self.answer(vec![0x71, invoke, reason]);
                    return Ok(());
                }
                if quirks.fail_rpm.load(Ordering::Relaxed) {
                    let class = ErrorClass::OBJECT.to_raw() as u8;
                    let code = ErrorCode::UNKNOWN_OBJECT.to_raw() as u8;
                    self.answer(vec![0x50, invoke, RPM, 0x91, class, 0x91, code]);
                    return Ok(());
                }
            }
            if service == SUBSCRIBE_COV && take(&quirks.swallow_cov) {
                return Ok(());
            }
        } else if self.quirks.muted.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }
}

pub(super) struct Lab {
    pub(super) hub: Hub<Link>,
    pub(super) engine: Arc<Engine<Link>>,
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
        device: LAB,
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
        let link = Link {
            inner: client_port,
            quirks: Arc::clone(&quirks),
            sent: Arc::clone(&sent),
            inject: Arc::new(Mutex::new(None)),
        };
        let client = BACnetClient::start(config, link).await.unwrap();
        client.add_device(LAB, &[2]).await.unwrap();
        let engine = Arc::new(Engine::new(
            Arc::new(client),
            Arc::new(RequestLimits::new(0)),
        ));
        let hub = Hub::new(Arc::clone(&engine), options).unwrap();
        Self {
            hub,
            engine,
            server,
            quirks,
            sent,
        }
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
        let Ok(engine) = Arc::try_unwrap(self.engine) else {
            panic!("the hub let go of the engine");
        };
        let Ok(mut client) = Arc::try_unwrap(engine.client) else {
            panic!("the engine holds the only client");
        };
        client.stop().await.unwrap();
        let mut server = self.server;
        server.stop().await.unwrap();
    }
}

/// Let the lab's tasks run: paused time moves on only when all are idle.
pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}
