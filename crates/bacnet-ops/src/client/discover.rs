//! Discovery through the client: where a Who-Is or a Who-Has goes
//! ([`Scope`]), where a handle made from a bare instance asks for its
//! device ([`ResolveScope`]), one discovery ([`Discover`]), Who-Is under the
//! rate cap, and the client's discovery table.

use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast, DeviceEvent, DiscoveredRouter};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use super::Client;
use crate::device::DeviceAddress;
use crate::error::OpsError;
use crate::target::{self, AddressStyle, Target, VmacLookup};

/// Why a station address in the target grammar is refused where one
/// station on this network is wanted.
const NOT_A_STATION: &str =
    "a directed Who-Is needs a station address, not a device instance or routed target";

/// How long a [`Discover`] listens for answers unless it says otherwise.
pub const DISCOVER_WAIT: Duration = Duration::from_secs(3);

/// Where a Who-Is ([`Discover`]) or a Who-Has ([`Find`](super::Find))
/// goes. The default is this network alone: a broadcast that reaches every
/// device on a large routed site is a choice, made by naming one network or
/// every network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

/// Where a handle made from a bare device instance (`1234`) asks for its
/// device when no address in the discovery table claims it: one Who-Is for
/// that instance ([`Client::with_resolve_scope`]). This network alone by
/// default; a target that names its network (`5:1234`) asks that network
/// whatever this says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResolveScope {
    /// This network alone: a local broadcast.
    #[default]
    Local,
    /// One remote network, 1 to 65534, through its router; 0 and 65535
    /// are taken as [`Local`](Self::Local) and [`Global`](Self::Global)
    /// ([`normalized`](Self::normalized)).
    Network(u16),
    /// Every network: a global broadcast.
    Global,
}

impl From<ResolveScope> for Scope {
    fn from(scope: ResolveScope) -> Self {
        match scope {
            ResolveScope::Local => Self::Local,
            ResolveScope::Network(network) => Self::Network(network),
            ResolveScope::Global => Self::Global,
        }
    }
}

impl ResolveScope {
    /// To `network`: this one for 0, every one for 65535, one remote
    /// network otherwise.
    pub(crate) fn on(network: u16) -> Self {
        Self::Network(network).normalized()
    }

    /// The scope as a Who-Is goes: `Network(0)` is this network
    /// ([`Local`](Self::Local)) and `Network(65535)` every network
    /// ([`Global`](Self::Global)); any other is itself.
    /// [`Client::with_resolve_scope`] stores a setting this way.
    #[must_use]
    pub fn normalized(self) -> Self {
        match self {
            Self::Network(0) => Self::Local,
            Self::Network(u16::MAX) => Self::Global,
            scope => scope,
        }
    }
}

/// Which request a [`Scope`] sends, for the words a refusal uses: the
/// service, and how the request that carries it names a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    /// A Who-Is, from a [`Discover`].
    WhoIs,
    /// A Who-Has, from a [`Find`](super::Find).
    WhoHas,
}

impl Ask {
    fn service(self) -> &'static str {
        match self {
            Self::WhoIs => "Who-Is",
            Self::WhoHas => "Who-Has",
        }
    }

    /// The call that sends this request to `scope`, for a refusal to name.
    fn naming(self, scope: &Scope) -> String {
        match (self, scope) {
            (Self::WhoIs, Scope::Local) => "Discover::local()".into(),
            (Self::WhoIs, Scope::Global) => "Discover::global()".into(),
            (Self::WhoIs, Scope::Network(network)) => format!("Discover::network({network})"),
            (Self::WhoIs, Scope::Directed(_)) => "Discover::directed(..)".into(),
            (Self::WhoHas, Scope::Directed(_)) => "Find::within(Scope::Directed(..))".into(),
            (Self::WhoHas, scope) => format!("Find::within(Scope::{scope:?})"),
        }
    }
}

impl Scope {
    /// The network this request goes out on, for its rate slot: a global
    /// one reaches every network and takes the global slot alone.
    pub(crate) fn network(&self) -> Option<u16> {
        match self {
            Self::Local | Self::Directed(_) => Some(0),
            Self::Network(network) => Some(*network),
            Self::Global => None,
        }
    }

    /// The scope of the request `ask` names, checked before anything is
    /// sent against `client`: the station's MAC for a directed one.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for a network outside 1 to 65534, and for a
    /// directed address on another network, saying which scope to use
    /// instead.
    pub(crate) fn station<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        ask: Ask,
    ) -> Result<Option<Vec<u8>>, OpsError> {
        let refused =
            |network: u16, reason: String| OpsError::parse("network", &network.to_string(), reason);
        match self {
            Self::Network(0) => Err(refused(
                0,
                format!(
                    "network 0 is this network: leave the network out, or use a local scope \
                     ({})",
                    ask.naming(&Self::Local)
                ),
            )),
            Self::Network(u16::MAX) => Err(refused(
                u16::MAX,
                format!(
                    "network 65535 is every network: use a global scope ({})",
                    ask.naming(&Self::Global)
                ),
            )),
            Self::Directed(address) => {
                let local = address.clone().localized(client.network_number());
                if local.is_local() {
                    return Ok(Some(local.mac().to_vec()));
                }
                let network = local.network();
                Err(OpsError::parse(
                    "address",
                    &address.to_string(),
                    format!(
                        "{address} is on network {network}, and a directed {} goes to one \
                         station on this network: {} asks that network's devices",
                        ask.service(),
                        ask.naming(&Self::Network(network)),
                    ),
                ))
            }
            Self::Local | Self::Network(_) | Self::Global => Ok(None),
        }
    }

    /// Whether a device at `address` is one this scope's Who-Is asked,
    /// `own` being the client's network number once known: any for a
    /// global one; for a local one, a station on this network; for a
    /// remote network, a station on it (this network when it is `own`);
    /// for a directed one, that station. An I-Am from elsewhere during a
    /// discovery's wait, unasked or the answer to another's Who-Is, is no
    /// answer to it, as for a Who-Has (`Scope::answered_by`, #2596).
    pub(crate) fn covers(&self, address: &DeviceAddress, own: Option<u16>) -> bool {
        let address = address.clone().localized(own);
        let network = (!address.is_local()).then(|| address.network());
        match self {
            Self::Local => Broadcast::Local.answered_from(network, own),
            Self::Network(n) => Broadcast::Network(*n).answered_from(network, own),
            Self::Global => Broadcast::Global.answered_from(network, own),
            Self::Directed(station) => address == station.clone().localized(own),
        }
    }

    /// Send the Who-Is, once checked ([`station`](Self::station)).
    pub(crate) async fn send<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        station: Option<&[u8]>,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        match (self, station) {
            (Self::Local, _) => client
                .who_is(Broadcast::Local, range)
                .await
                .map_err(|error| to_local_broadcast(client, error)),
            (Self::Network(network), _) => client.who_is(Broadcast::Network(*network), range).await,
            (Self::Directed(_), Some(mac)) => client.who_is_directed(mac, range).await,
            (Self::Directed(_), None) => unreachable!("a directed Who-Is has its station"),
            (Self::Global, _) => client.who_is(Broadcast::Global, range).await,
        }
    }
}

/// A local broadcast the link failed to send, saying where it was going.
/// The operating system's own text names no address ("No route to host"),
/// and for a B/IP client the address is a setting the user can fix: the
/// broadcast address the client was given.
#[derive(Debug)]
struct LocalBroadcastFailed {
    to: std::net::SocketAddrV4,
    source: std::io::Error,
}

impl std::fmt::Display for LocalBroadcastFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}, sending the local broadcast to {} (the client's broadcast address)",
            self.source, self.to
        )
    }
}

impl std::error::Error for LocalBroadcastFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// `error`, from a local broadcast on `client`'s link: a transport error
/// on a B/IP link names the broadcast address it was sent to and keeps the
/// operating system's error as its source, with its kind. Any other error
/// is as it came.
pub(crate) fn to_local_broadcast<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    error: Error,
) -> Error {
    match (error, client.transport().bip_broadcast_endpoint()) {
        (Error::Transport(source), Some(to)) => Error::Transport(std::io::Error::new(
            source.kind(),
            LocalBroadcastFailed { to, source },
        )),
        (error, _) => error,
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

    /// The station at `address` on this network, by unicast. From text in
    /// the target grammar, [`Client::station_mac`] finds the MAC:
    ///
    /// ```no_run
    /// use bacnet_ops::{Client, DeviceAddress, Discover};
    /// # async fn example(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let station = DeviceAddress::local(&client.station_mac("192.168.1.20").await?)?;
    /// let found = client.discover(Discover::directed(station)).await?;
    /// # let _ = found; Ok(()) }
    /// ```
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
    pub(crate) fn instances(&self) -> Result<Option<DeviceInstanceRange>, OpsError> {
        instances(self.range.as_ref())
    }
}

/// A request's device instance range, checked: both limits within 0 to
/// 4194303, the low one no greater than the high one.
pub(crate) fn instances(
    range: Option<&RangeInclusive<u32>>,
) -> Result<Option<DeviceInstanceRange>, OpsError> {
    range
        .map(|range| {
            let (low, high) = (*range.start(), *range.end());
            DeviceInstanceRange::new(low, high).map_err(|error| {
                let reason = match error {
                    Error::Range(refused) => refused.message().to_string(),
                    other => other.to_string(),
                };
                OpsError::parse("range", &format!("{low}-{high}"), reason)
            })
        })
        .transpose()
}

impl<T: TransportPort + 'static> Client<T> {
    /// Send one Who-Is to `scope`, for the devices in `range` (all for
    /// `None`), under the rate cap. The answers land in the discovery table
    /// ([`devices`](Self::devices)) as they arrive.
    pub(crate) async fn who_is(
        &self,
        scope: &Scope,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), OpsError> {
        let station = scope.station(self.low_level(), Ask::WhoIs)?;
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
        sent.map_err(OpsError::from)
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
    ) -> Result<Vec<DiscoveredRouter>, OpsError> {
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
        found.map_err(OpsError::from)
    }

    /// The link MAC of a station address in the target grammar, checked
    /// without sending anything but the Virtual-Address-Resolution a
    /// B/IPv6 address needs: a device instance or routed target is no
    /// station address.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that names no station on this link (a
    /// B/IPv6 address on another link among them), and
    /// [`OpsError::UnknownDevice`] for a B/IPv6 address whose node didn't
    /// say its VMAC.
    pub async fn station_mac(&self, address: &str) -> Result<Vec<u8>, OpsError> {
        let transport = self.low_level().transport();
        let parse = |reason: String| OpsError::parse("address", address, reason);
        match target::parse_target_on(transport, address).map_err(parse)? {
            Target::Mac(mac) => Ok(mac),
            Target::Bip6(peer) => {
                target::bip6_vmac(transport, peer)
                    .await
                    .map_err(|why| match why {
                        VmacLookup::NoAnswer { .. } => OpsError::UnknownDevice {
                            target: address.to_string(),
                            asked: None,
                        },
                        why => parse(why.to_string()),
                    })
            }
            Target::Instance(_) | Target::Routed { .. } | Target::RoutedMac { .. } => {
                Err(parse(NOT_A_STATION.to_string()))
            }
        }
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
