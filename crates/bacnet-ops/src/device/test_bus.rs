//! An in-memory link for the handle tests, on the shared test transport:
//! every port hears every broadcast, a unicast reaches the port with that
//! MAC, a silenced port hears nothing, and a test can send an I-Am from any
//! MAC. MACs are six octets shaped like B/IP addresses, so they read as
//! `10.0.0.N:47808`.

use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::device::DeviceAddress;

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
}

impl Bus {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A port at station `n`.
    pub(super) fn port(self: &Arc<Self>, n: u8) -> TestTransport {
        let source = MacAddr::from_slice(&mac(n));
        let (inbox, rx) = mpsc::channel(256);
        self.ports.lock().unwrap().push(Port {
            mac: source.clone(),
            inbox,
            silent: false,
        });
        let bus = Arc::clone(self);
        TestTransport::builder()
            .local_mac(&source)
            .inbound(rx)
            .on_send(move |frame| {
                let to = (!frame.broadcast).then_some(frame.mac.as_slice());
                bus.deliver(&source, &frame.npdu, to);
                async { Ok(()) }
            })
            .build()
    }

    /// Station `n` hears nothing from now on, as a device gone away.
    pub(super) fn silence(&self, n: u8) {
        for port in self.ports.lock().unwrap().iter_mut() {
            if port.mac.as_slice() == mac(n) {
                port.silent = true;
            }
        }
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
