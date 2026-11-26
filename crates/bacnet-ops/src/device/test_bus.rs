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
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_has::IHaveRequest;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, TestTransport, TestTransportBuilder};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::client::Client;
use crate::device::{Device, DeviceAddress};
use crate::error::OpsError;
use crate::limits::RequestLimits;

/// The instance the tests' devices share.
pub(crate) const N: u32 = 1001;

/// A client that times out in one second, with no retries.
pub(crate) fn config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    }
}

/// A client on `port`, and its sends.
pub(crate) async fn client_on(port: TestTransport) -> (Client<TestTransport>, SendLog) {
    let sent = port.sent();
    let low = BACnetClient::start(config(), port).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    (client, sent)
}

/// A client at station 1 of `bus`, and its sends.
pub(crate) async fn client(bus: &Arc<Bus>) -> (Client<TestTransport>, SendLog) {
    client_on(bus.port(1)).await
}

/// The analog input's name, as `device` reads it.
pub(crate) async fn name_of(device: &Device<TestTransport>) -> Result<String, OpsError> {
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let value = device.read((ai, PropertyIdentifier::OBJECT_NAME)).await?;
    match value.as_str() {
        Some(name) => Ok(name.to_string()),
        None => panic!("a name: {value:?}"),
    }
}

/// Let the link's frames land.
pub(crate) async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

pub(crate) async fn stop(servers: Vec<BACnetServer<TestTransport>>) {
    for mut server in servers {
        server.stop().await.unwrap();
    }
}

/// The MAC of station `n` on the bus.
pub(crate) fn mac(n: u8) -> [u8; 6] {
    [10, 0, 0, n, 0xBA, 0xC0]
}

/// Station `n`'s address.
pub(crate) fn at(n: u8) -> DeviceAddress {
    DeviceAddress::local(&mac(n)).unwrap()
}

struct Port {
    mac: MacAddr,
    inbox: mpsc::Sender<ReceivedNpdu>,
    silent: bool,
}

/// The link.
#[derive(Default)]
pub(crate) struct Bus {
    ports: Mutex<Vec<Port>>,
    /// Stations a unicast to fails at the sender.
    broken: Mutex<HashSet<u8>>,
    /// Unicasts to each station still to be lost on the way, as one lost
    /// datagram is.
    losing: Mutex<std::collections::HashMap<u8, usize>>,
    /// The Time_Of_Device_Restart each station answers a ReadProperty of
    /// with, in place of its server, which has none.
    restart_times: Mutex<std::collections::HashMap<u8, Vec<u8>>>,
    /// Confirmed requests to each station still to draw a Reject or an
    /// Abort from it in place of its server's answer.
    turning: Mutex<std::collections::HashMap<u8, (usize, TurnAway)>>,
}

/// How a station turns a confirmed request away: it says nothing of which
/// device it is either way.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TurnAway {
    /// A Reject PDU, reason other.
    Reject,
    /// An Abort PDU from the server, reason out-of-resources.
    Abort,
}

impl Bus {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A port at station `n`.
    pub(crate) fn port(self: &Arc<Self>, n: u8) -> TestTransport {
        self.port_with(n, |builder| builder)
    }

    /// A port at station `n` that takes the network number controls, so a
    /// client on it learns its network's number from a router.
    pub(crate) fn numbered_port(self: &Arc<Self>, n: u8) -> TestTransport {
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
                if !broken
                    && !bus.lost(to)
                    && !bus.restart_time_answered(&source, &frame.npdu, to)
                    && !bus.turned_away(&source, &frame.npdu, to)
                {
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

    /// Station `n` answers a ReadProperty of its Device object's
    /// Time_Of_Device_Restart with `time` (an Unsigned, which tests compare),
    /// or as its server does, which has none, for `None`.
    pub(crate) fn restart_time(&self, n: u8, time: Option<u8>) {
        let mut times = self.restart_times.lock().unwrap();
        match time {
            Some(time) => times.insert(n, vec![0x21, time]),
            None => times.remove(&n),
        };
    }

    /// Answer `npdu` from `from` to `to` here when it reads the
    /// Time_Of_Device_Restart of a station given one: whether it did.
    fn restart_time_answered(&self, from: &MacAddr, npdu: &[u8], to: Option<&[u8]>) -> bool {
        let Some(n) = to.and_then(|to| (0..=u8::MAX).find(|n| mac(*n) == to)) else {
            return false;
        };
        let Some(time) = self.restart_times.lock().unwrap().get(&n).cloned() else {
            return false;
        };
        let Ok(decoded) = decode_npdu(Bytes::copy_from_slice(npdu)) else {
            return false;
        };
        let apdu = &decoded.payload;
        let read = ConfirmedServiceChoice::READ_PROPERTY.to_raw();
        if apdu.len() < 4 || apdu[0] >> 4 != 0 || apdu[3] != read {
            return false;
        }
        let Ok(request) = ReadPropertyRequest::decode(&apdu[4..]) else {
            return false;
        };
        if request.property_identifier != PropertyIdentifier::TIME_OF_DEVICE_RESTART {
            return false;
        }
        let mut ack = BytesMut::from(&[1u8, 0, 0x30, apdu[2], read][..]);
        ReadPropertyACK {
            object_identifier: request.object_identifier,
            property_identifier: request.property_identifier,
            property_array_index: None,
            property_value: time,
        }
        .encode(&mut ack)
        .unwrap();
        self.deliver(&MacAddr::from_slice(&mac(n)), &ack, Some(from.as_slice()));
        true
    }

    /// The next `count` confirmed requests to station `n` draw `how` from
    /// it in place of its server's answer, while it answers the rest.
    pub(crate) fn turn_away(&self, n: u8, count: usize, how: TurnAway) {
        self.turning.lock().unwrap().insert(n, (count, how));
    }

    /// Answer `npdu` from `from` to `to` here when it is a confirmed request
    /// to a station turning requests away: whether it did.
    fn turned_away(&self, from: &MacAddr, npdu: &[u8], to: Option<&[u8]>) -> bool {
        let Some(n) = to.and_then(|to| (0..=u8::MAX).find(|n| mac(*n) == to)) else {
            return false;
        };
        let Ok(decoded) = decode_npdu(Bytes::copy_from_slice(npdu)) else {
            return false;
        };
        let apdu = &decoded.payload;
        if apdu.len() < 4 || apdu[0] >> 4 != 0 {
            return false;
        }
        let how = {
            let mut turning = self.turning.lock().unwrap();
            match turning.get_mut(&n) {
                Some((left, how)) if *left > 0 => {
                    *left -= 1;
                    *how
                }
                _ => return false,
            }
        };
        let answer = match how {
            TurnAway::Reject => [1u8, 0, 0x60, apdu[2], 0],
            TurnAway::Abort => [1u8, 0, 0x71, apdu[2], 9],
        };
        self.deliver(
            &MacAddr::from_slice(&mac(n)),
            &answer,
            Some(from.as_slice()),
        );
        true
    }

    /// Send `npdu` from station `n` to station `to`, as that station would.
    pub(crate) fn send_from(&self, n: u8, npdu: &[u8], to: u8) {
        self.deliver(&MacAddr::from_slice(&mac(n)), npdu, Some(&mac(to)));
    }

    /// Station `n` hears nothing from now on, as a device gone away.
    pub(crate) fn silence(&self, n: u8) {
        self.set_silent(n, true);
    }

    /// Station `n` hears again.
    pub(crate) fn unsilence(&self, n: u8) {
        self.set_silent(n, false);
    }

    fn set_silent(&self, n: u8, silent: bool) {
        for port in self.ports.lock().unwrap().iter_mut() {
            if port.mac.as_slice() == mac(n) {
                port.silent = silent;
            }
        }
    }

    /// The next `count` unicasts to station `n` are lost on the way: one
    /// request it misses, as a lost datagram, while it answers the rest.
    pub(crate) fn lose(&self, n: u8, count: usize) {
        self.losing.lock().unwrap().insert(n, count);
    }

    /// Whether a unicast to `to` is lost, counting it.
    fn lost(&self, to: Option<&[u8]>) -> bool {
        let Some(n) = to.and_then(|to| (0..=u8::MAX).find(|n| mac(*n) == to)) else {
            return false;
        };
        let mut losing = self.losing.lock().unwrap();
        match losing.get_mut(&n) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            }
            _ => false,
        }
    }

    /// Unicasts to station `n` fail at the sender from now on, as a local
    /// link failure: nothing reaches the station.
    pub(crate) fn break_link(&self, n: u8) {
        self.broken.lock().unwrap().insert(n);
    }

    /// A router at station `n` with no route anywhere: every NPDU it hears
    /// for one remote network draws its reject, reason 1 naming that
    /// network, sent back to the station that sent it.
    pub(super) fn rejecter(self: &Arc<Self>, n: u8) {
        let station = MacAddr::from_slice(&mac(n));
        let (inbox, mut rx) = mpsc::channel::<ReceivedNpdu>(256);
        self.ports.lock().unwrap().push(Port {
            mac: station.clone(),
            inbox,
            silent: false,
        });
        let bus = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                let Ok(npdu) = decode_npdu(frame.npdu.clone()) else {
                    continue;
                };
                let Some(network) = npdu.destination.map(|to| to.network) else {
                    continue;
                };
                if network == 0xFFFF {
                    continue;
                }
                let [high, low] = network.to_be_bytes();
                let reject = [0x01, 0x80, 0x03, 0x01, high, low];
                bus.deliver(&station, &reject, Some(&frame.source_mac));
            }
        });
    }

    /// Broadcast an I-Am for `instance` from station `n`.
    pub(crate) fn i_am(&self, n: u8, instance: u32) {
        self.i_am_from(n, None, instance);
    }

    /// Inject an I-Am from a local station or through a router with SNET/SADR.
    pub(super) fn i_am_from(&self, n: u8, routed: Option<(u16, &[u8])>, instance: u32) {
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
            source: routed.map(|(network, mac)| NpduAddress {
                network,
                mac_address: MacAddr::from_slice(mac),
            }),
            payload: payload.freeze(),
            ..Npdu::default()
        };
        encode_npdu(&mut npdu, &frame).unwrap();
        self.deliver(&MacAddr::from_slice(&mac(n)), &npdu, None);
    }

    /// Broadcast an I-Have from station `n`: device `instance` has analog
    /// input 1, named `name`. With `routed`, it carries that SNET and SADR,
    /// as a router at station `n` passes one on.
    pub(super) fn i_have(&self, n: u8, routed: Option<(u16, &[u8])>, instance: u32, name: &str) {
        let mut service = BytesMut::new();
        IHaveRequest {
            device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            object_name: name.into(),
        }
        .encode(&mut service)
        .unwrap();
        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_HAVE,
            service_request: service.freeze(),
        });
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &apdu).unwrap();
        let mut npdu = BytesMut::new();
        let frame = Npdu {
            source: routed.map(|(network, mac)| NpduAddress {
                network,
                mac_address: MacAddr::from_slice(mac),
            }),
            payload: payload.freeze(),
            ..Npdu::default()
        };
        encode_npdu(&mut npdu, &frame).unwrap();
        self.deliver(&MacAddr::from_slice(&mac(n)), &npdu, None);
    }

    /// Broadcast a Network-Number-Is from station `n`: this network is
    /// `number`, as configured there.
    pub(super) fn number_is(&self, n: u8, number: u16) {
        let [high, low] = number.to_be_bytes();
        // NPDU version 1, a network message, Network-Number-Is (0x13), the
        // number, and 1 for configured.
        let npdu = [0x01, 0x80, 0x13, high, low, 1];
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
pub(crate) async fn server(
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

/// Who-Is requests in `log`.
pub(crate) fn who_is(log: &SendLog) -> usize {
    log.broadcasts()
        .iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::WHO_IS)
        })
        .count()
}

/// Confirmed requests of `service` in `log`, to `to`.
pub(crate) fn requests(log: &SendLog, service: ConfirmedServiceChoice, to: u8) -> usize {
    log.unicasts()
        .iter()
        .filter(|frame| frame.mac.as_slice() == mac(to))
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::ConfirmedRequest(request)
                if request.service_choice == service)
        })
        .count()
}
