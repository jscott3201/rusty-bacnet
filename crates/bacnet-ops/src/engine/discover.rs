//! Discovery through the engine: Who-Is under the rate cap, and the
//! client's discovery table.

use bacnet_client::client::{DeviceEvent, DiscoveredRouter};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::Engine;
use crate::target::{self, AddressStyle};

/// Why a directed Who-Is refuses a device instance or a routed target.
const NOT_A_STATION: &str =
    "a directed Who-Is needs a station address, not a device instance or routed target";

/// Where a Who-Is goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhoIsScope<'a> {
    /// Broadcast to this network alone.
    Local,
    /// A broadcast on one remote network, through its router.
    Network(u16),
    /// One station, by its address in the target grammar
    /// ([`target::address_mac`]).
    Directed(&'a str),
    /// A global broadcast, which every router passes on.
    Global,
}

impl<T: TransportPort + 'static> Engine<T> {
    /// Send one Who-Is to `scope`, for the devices in `range` (all for
    /// `None`), under the rate cap. The answers land in the discovery table
    /// ([`devices`](Self::devices)) as they arrive.
    pub async fn who_is(
        &self,
        scope: WhoIsScope<'_>,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        let mac = match scope {
            WhoIsScope::Directed(address) => Some(
                target::address_mac(self.client.transport(), address, NOT_A_STATION)
                    .await
                    .map_err(Error::Encoding)?,
            ),
            _ => None,
        };
        // The network the Who-Is goes out on takes its slot, so a slow one
        // is paced; a global one reaches every network and takes the
        // global slot alone.
        let network = match scope {
            WhoIsScope::Local | WhoIsScope::Directed(_) => Some(0),
            WhoIsScope::Network(network) => Some(network),
            WhoIsScope::Global => None,
        };
        self.limits.slot(network).await;
        let sent = match (scope, mac) {
            (WhoIsScope::Local, _) => {
                let mut buf = BytesMut::new();
                WhoIsRequest { range }.encode(&mut buf);
                self.client
                    .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
                    .await
            }
            (WhoIsScope::Network(network), _) => self.client.who_is_network(network, range).await,
            (WhoIsScope::Directed(_), Some(mac)) => self.client.who_is_directed(&mac, range).await,
            (WhoIsScope::Directed(_), None) => unreachable!("a directed Who-Is has its address"),
            (WhoIsScope::Global, _) => self.client.who_is(range).await,
        };
        // An unconfirmed send that fails never left: the caller isn't
        // charged for it.
        if sent.is_err() {
            crate::limits::refund();
        }
        sent
    }

    /// Broadcast one Who-Is-Router-To-Network on this network, for
    /// `network` or every network (`None`), under the rate cap, then
    /// collect the I-Am-Router-To-Network answers heard for `wait`, or
    /// until `deadline` if that comes first (counted after the wait for
    /// the rate slot): one entry per router, with the networks it reaches.
    /// No answer is an empty list.
    pub async fn discover_routers(
        &self,
        network: Option<u16>,
        wait: std::time::Duration,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<Vec<DiscoveredRouter>, Error> {
        self.limits.slot(Some(0)).await;
        let wait = match deadline {
            Some(deadline) => {
                wait.min(deadline.saturating_duration_since(tokio::time::Instant::now()))
            }
            None => wait,
        };
        let found = self.client.discover_routers(network, wait).await;

        // A refused send never left, so it isn't charged.
        if found.is_err() {
            crate::limits::refund();
        }
        found
    }

    /// The link MAC of a station address in the target grammar, checked
    /// without sending anything: a device instance or routed target is no
    /// station address.
    pub async fn station_mac(&self, address: &str) -> Result<Vec<u8>, Error> {
        target::address_mac(self.client.transport(), address, NOT_A_STATION)
            .await
            .map_err(Error::Encoding)
    }

    /// Every device in the client's discovery table.
    pub async fn devices(&self) -> Vec<DiscoveredDevice> {
        self.client.discovered_devices().await
    }

    /// One device from the discovery table.
    pub async fn device(&self, instance: u32) -> Option<DiscoveredDevice> {
        self.client.get_device(instance).await
    }

    /// A receiver of the discovery table's changes: devices found,
    /// refreshed and lost. Lossy under load, like every client channel; the
    /// table ([`devices`](Self::devices)) stays the truth.
    pub fn device_events(&self) -> tokio::sync::broadcast::Receiver<DeviceEvent> {
        self.client.device_events()
    }

    /// How this client's link writes addresses.
    pub fn address_style(&self) -> AddressStyle {
        AddressStyle::of(self.client.transport())
    }
}
