//! Discovery through the client: where a Who-Is goes ([`Scope`]), one
//! discovery ([`Discover`]), Who-Is under the rate cap, and the client's
//! discovery table.

use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEvent, DiscoveredRouter};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::Client;
use crate::device::DeviceAddress;
use crate::target::{self, AddressStyle};

/// Why a station address in the target grammar is refused where one
/// station on this network is wanted.
const NOT_A_STATION: &str =
    "a directed Who-Is needs a station address, not a device instance or routed target";

/// How long a [`Discover`] listens for answers unless it says otherwise.
pub const DISCOVER_WAIT: Duration = Duration::from_secs(3);

/// Where a Who-Is goes. The default is this network alone: a broadcast that
/// reaches every device on a large routed site is a choice, made by naming
/// one network or every network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// A broadcast on this network alone, which no router passes on.
    #[default]
    Local,
    /// A broadcast on one remote network, 1 to 65534, through its router.
    /// This client's own network, once it has learned the number, is a
    /// local broadcast.
    Network(u16),
    /// A global broadcast, which every router passes on: every network.
    Global,
    /// One station on this network, by unicast. An address on this client's
    /// own network, once it knows the number, is the local station.
    Directed(DeviceAddress),
}

impl Scope {
    /// To `network` (this one for 0), or every network for `None`: how a
    /// handle asks for the device it resolves.
    pub(crate) fn on(network: Option<u16>) -> Self {
        match network {
            Some(0) => Self::Local,
            Some(network) => Self::Network(network),
            None => Self::Global,
        }
    }

    /// Send one Who-Is here over `client`, for the devices in `range` (all
    /// for `None`), outside any request limits: for a front end that works
    /// on the low-level client. A [`Client`] sends through
    /// [`Client::discover`], under its limits. The answers land in the
    /// client's discovery table as they arrive.
    ///
    /// # Errors
    ///
    /// Before anything is sent, [`Error::OutOfRange`] for a network outside
    /// 1 to 65534 or a directed address on another network; then the
    /// transport's error.
    pub async fn who_is<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        let to = self.station(client)?;
        self.send(client, to.as_deref(), range).await
    }

    /// The network this Who-Is goes out on, for its rate slot: a global one
    /// reaches every network and takes the global slot alone.
    fn network(&self) -> Option<u16> {
        match self {
            Self::Local | Self::Directed(_) => Some(0),
            Self::Network(network) => Some(*network),
            Self::Global => None,
        }
    }

    /// The scope checked before anything is sent: the station's MAC for a
    /// directed one.
    fn station<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
    ) -> Result<Option<Vec<u8>>, Error> {
        match self {
            Self::Network(network) if !(1..=65534).contains(network) => Err(Error::OutOfRange(
                format!("a Who-Is goes to network 1 to 65534, not {network}"),
            )),
            Self::Directed(address) => {
                let local = address.clone().localized(client.network_number());
                if local.is_local() {
                    Ok(Some(local.mac().to_vec()))
                } else {
                    Err(Error::OutOfRange(format!(
                        "a directed Who-Is goes to one station on this network, not {address}"
                    )))
                }
            }
            Self::Local | Self::Network(_) | Self::Global => Ok(None),
        }
    }

    /// Send the Who-Is, once checked ([`station`](Self::station)).
    async fn send<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        station: Option<&[u8]>,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        match (self, station) {
            (Self::Local, _) => {
                let mut buf = BytesMut::new();
                WhoIsRequest { range }.encode(&mut buf);
                client
                    .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
                    .await
            }
            (Self::Network(network), _) => client.who_is_network(*network, range).await,
            (Self::Directed(_), Some(mac)) => client.who_is_directed(mac, range).await,
            (Self::Directed(_), None) => unreachable!("a directed Who-Is has its station"),
            (Self::Global, _) => client.who_is(range).await,
        }
    }
}

/// One discovery for [`Client::discover`]: where its Who-Is goes, the
/// device instances it asks, and how long it listens for the answers.
/// [`Discover::default`] asks every device on this network alone and
/// listens for [`DISCOVER_WAIT`].
///
/// ```
/// use std::time::Duration;
/// use bacnet_ops::Discover;
///
/// let nearby = Discover::local().range(1000..=2000);
/// let behind_a_router = Discover::network(5).wait(Duration::from_secs(5));
/// let everywhere = Discover::global();
/// # let _ = (nearby, behind_a_router, everywhere);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discover {
    pub(crate) scope: Scope,
    range: Option<RangeInclusive<u32>>,
    pub(crate) wait: Duration,
}

impl Default for Discover {
    fn default() -> Self {
        Self::from(Scope::Local)
    }
}

impl From<Scope> for Discover {
    /// A discovery of every device in `scope`, listening for
    /// [`DISCOVER_WAIT`].
    fn from(scope: Scope) -> Self {
        Self {
            scope,
            range: None,
            wait: DISCOVER_WAIT,
        }
    }
}

impl Discover {
    /// The devices on this network alone: a local broadcast.
    pub fn local() -> Self {
        Self::from(Scope::Local)
    }

    /// The devices on the remote `network` (1 to 65534), through its router.
    pub fn network(network: u16) -> Self {
        Self::from(Scope::Network(network))
    }

    /// The devices on every network: a global broadcast, which every router
    /// passes on.
    pub fn global() -> Self {
        Self::from(Scope::Global)
    }

    /// The station at `address` on this network, by unicast.
    pub fn directed(address: DeviceAddress) -> Self {
        Self::from(Scope::Directed(address))
    }

    /// Ask only the devices whose instance is in `instances` (every device
    /// by default). Checked when the discovery runs: an empty range, or
    /// one past instance 4194303, fails it before anything is sent.
    #[must_use]
    pub fn range(mut self, instances: RangeInclusive<u32>) -> Self {
        self.range = Some(instances);
        self
    }

    /// Listen for answers for `wait` ([`DISCOVER_WAIT`] by default).
    #[must_use]
    pub fn wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Where the Who-Is goes.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The instance range asked, checked.
    pub(crate) fn instances(&self) -> Result<Option<DeviceInstanceRange>, Error> {
        self.range
            .as_ref()
            .map(|range| DeviceInstanceRange::new(*range.start(), *range.end()))
            .transpose()
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// Send one Who-Is to `scope`, for the devices in `range` (all for
    /// `None`), under the rate cap. The answers land in the discovery table
    /// ([`devices`](Self::devices)) as they arrive.
    pub(crate) async fn who_is(
        &self,
        scope: &Scope,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        let station = scope.station(self.low_level())?;
        // The network the Who-Is goes out on takes its slot, so a slow one
        // is paced; a global one takes the global slot alone.
        self.limits().slot(scope.network()).await;
        let sent = scope
            .send(self.low_level(), station.as_deref(), range)
            .await;
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
        self.limits().slot(Some(0)).await;
        let wait = match deadline {
            Some(deadline) => {
                wait.min(deadline.saturating_duration_since(tokio::time::Instant::now()))
            }
            None => wait,
        };
        let found = self.low_level().discover_routers(network, wait).await;

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
        target::address_mac(self.low_level().transport(), address, NOT_A_STATION)
            .await
            .map_err(Error::Encoding)
    }

    /// Every device in the client's discovery table.
    pub async fn devices(&self) -> Vec<DiscoveredDevice> {
        self.low_level().discovered_devices().await
    }

    /// A receiver of the discovery table's changes: devices found,
    /// refreshed and lost. Lossy under load, like every client channel; the
    /// table ([`devices`](Self::devices)) stays the truth.
    pub fn device_events(&self) -> tokio::sync::broadcast::Receiver<DeviceEvent> {
        self.low_level().device_events()
    }

    /// How this client's link writes addresses.
    pub fn address_style(&self) -> AddressStyle {
        AddressStyle::of(self.low_level().transport())
    }
}
