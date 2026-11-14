//! An in-memory link for the handle tests, on the shared test transport:
//! every port hears every broadcast, a unicast reaches the port with that
//! MAC, a silenced port hears nothing, a send to a broken station fails at
//! the sender as a transport error, and a test can send an I-Am from any
//! MAC. MACs are six octets shaped like B/IP addresses, so they read as
//! `10.0.0.N:47808`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, TestTransport, TestTransportBuilder};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::client::Client;
use crate::device::{Device, DeviceAddress};
use crate::format::decode_all;
use crate::limits::RequestLimits;

/// The instance the tests' devices share.
pub(super) const N: u32 = 1001;

/// A client that times out in one second, with no retries.
pub(super) fn config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    }
}

/// A client on `port`, and its sends.
pub(super) async fn client_on(port: TestTransport) -> (Client<TestTransport>, SendLog) {
    let sent = port.sent();
    let low = BACnetClient::start(config(), port).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    (client, sent)
}

/// A client at station 1 of `bus`, and its sends.
pub(super) async fn client(bus: &Arc<Bus>) -> (Client<TestTransport>, SendLog) {
    client_on(bus.port(1)).await
}

/// The analog input's name, as `device` reads it.
pub(super) async fn name_of(device: &Device<TestTransport>) -> Result<String, Error> {
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let data = device
        .read_property(ai, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    match decode_all(&data).as_deref() {
        Some([PropertyValue::CharacterString(name)]) => Ok(name.clone()),
        other => panic!("a name: {other:?}"),
    }
}

/// Let the link's frames land.
pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

pub(super) async fn stop(servers: Vec<BACnetServer<TestTransport>>) {
    for mut server in servers {
        server.stop().await.unwrap();
    }
}

/// The MAC of station `n` on the bus.
pub(super) fn mac(n: u8) -> [u8; 6] {
    [10, 0, 0, n, 0xBA, 0xC0]
}

/// Station `n`'s address.
pub(super) fn at(n: u8) -> DeviceAddress {
    DeviceAddress::local(&mac(n)).unwrap()
}

struct Port {
    mac: MacAddr,
    inbox: mpsc::Sender<ReceivedNpdu>,
    silent: bool,
}

/// The link.
#[derive(Default)]
pub(super) struct Bus {
    ports: Mutex<Vec<Port>>,
    /// Stations a unicast to fails at the sender.
    broken: Mutex<HashSet<u8>>,
}

impl Bus {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A port at station `n`.
    pub(super) fn port(self: &Arc<Self>, n: u8) -> TestTransport {
        self.port_with(n, |builder| builder)
    }

    /// A port at station `n` that takes the network number controls, so a
    /// client on it learns its network's number from a router.
    pub(super) fn numbered_port(self: &Arc<Self>, n: u8) -> TestTransport {
        self.port_with(n, TestTransportBuilder::number_controls)
    }

    fn port_with(
        self: &Arc<Self>,
        n: u8,
        more: impl FnOnce(TestTransportBuilder) -> TestTransportBuilder,
    ) -> TestTransport {
        let source = MacAddr::from_slice(&mac(n));
        let (inbox, rx) = mpsc::channel(256);
        self.ports.lock().unwrap().push(Port {
            mac: source.clone(),
            inbox,
            silent: false,
        });
        let bus = Arc::clone(self);
        let builder = TestTransport::builder()
            .local_mac(&source)
            .inbound(rx)
            .on_send(move |frame| {
                let to = (!frame.broadcast).then_some(frame.mac.as_slice());
                let broken = to.is_some_and(|to| {
                    let broken = bus.broken.lock().unwrap();
                    broken.iter().any(|n| mac(*n) == to)
                });
                if !broken {
                    bus.deliver(&source, &frame.npdu, to);
                }
                async move {
                    match broken {
                        true => Err(Error::Transport(std::io::ErrorKind::BrokenPipe.into())),
                        false => Ok(()),
                    }
                }
            });
        more(builder).build()
    }

    /// Station `n` hears nothing from now on, as a device gone away.
    pub(super) fn silence(&self, n: u8) {
        self.set_silent(n, true);
    }

    /// Station `n` hears again.
    pub(super) fn unsilence(&self, n: u8) {
        self.set_silent(n, false);
    }

    fn set_silent(&self, n: u8, silent: bool) {
        for port in self.ports.lock().unwrap().iter_mut() {
            if port.mac.as_slice() == mac(n) {
                port.silent = silent;
            }
        }
    }

    /// Unicasts to station `n` fail at the sender from now on, as a local
    /// link failure: nothing reaches the station.
    pub(super) fn break_link(&self, n: u8) {
        self.broken.lock().unwrap().insert(n);
    }

    /// Broadcast an I-Am for `instance` from station `n`.
    pub(super) fn i_am(&self, n: u8, instance: u32) {
        let mut service = BytesMut::new();
        IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            vendor_id: 42,
        }
        .encode(&mut service);
        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        });
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &apdu).unwrap();
        let mut npdu = BytesMut::new();
        let frame = Npdu {
            payload: payload.freeze(),
            ..Npdu::default()
        };
        encode_npdu(&mut npdu, &frame).unwrap();
        self.deliver(&MacAddr::from_slice(&mac(n)), &npdu, None);
    }

    fn deliver(&self, from: &MacAddr, npdu: &[u8], to: Option<&[u8]>) {
        for port in self.ports.lock().unwrap().iter() {
            let reaches = to.is_none_or(|to| port.mac.as_slice() == to);
            if port.mac != *from && reaches && !port.silent {
                let frame = ReceivedNpdu::unverified(
                    Bytes::copy_from_slice(npdu),
                    from.clone(),
                    to.is_none(),
                    Vec::new(),
                    None,
                );
                let _ = port.inbox.try_send(frame);
            }
        }
    }
}

/// A device `instance` with one analog input named `name`, on `port`.
pub(super) async fn server(
    port: TestTransport,
    instance: u32,
    name: &str,
) -> BACnetServer<TestTransport> {
    let identity = DeviceIdentity::new(instance, 42).unwrap();
    let ai = AnalogInputObject::new(1, name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![Box::new(ai)];
    let database = build_database_with_extra(&identity, objects).unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(port)
        .build()
        .await
        .unwrap()
}

/// Confirmed requests of `service` in `log`, to `to`.
pub(super) fn requests(log: &SendLog, service: ConfirmedServiceChoice, to: u8) -> usize {
    log.unicasts()
        .iter()
        .filter(|frame| frame.mac.as_slice() == mac(to))
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::ConfirmedRequest(request)
                if request.service_choice == service)
        })
        .count()
}
