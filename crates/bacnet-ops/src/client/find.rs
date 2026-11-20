//! Finding objects through the client (#2596): one Who-Has ([`Find`]) to
//! a [`Scope`], this network alone unless the request names another, and
//! the I-Haves that answer it ([`FoundObject`]).

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast, ReceivedIHave, Station, collect_answers};
use bacnet_services::who_has::WhoHasObject;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::Client;
use super::discover::{Ask, Scope, instances};
use crate::device::DeviceAddress;
use crate::error::OpsError;

/// How long a [`Find`] listens for answers unless it says otherwise.
pub const FIND_WAIT: Duration = Duration::from_secs(3);

/// One search for [`Client::find`]: the object, by name or by identifier;
/// where its Who-Has goes, this network alone unless
/// [`within`](Self::within) names another [`Scope`]; the device instances
/// it asks; and how long it listens for the answers ([`FIND_WAIT`]).
///
/// ```
/// use std::time::Duration;
/// use bacnet_ops::{Find, Scope};
/// use bacnet_types::enums::ObjectType;
/// use bacnet_types::primitives::ObjectIdentifier;
///
/// let nearby = Find::name("Zone Temp");
/// let behind_a_router = Find::name("Zone Temp").within(Scope::Network(5));
/// let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
/// let everywhere = Find::object(ai)
///     .within(Scope::Global)
///     .range(1000..=2000)
///     .wait(Duration::from_secs(5));
/// # let _ = (nearby, behind_a_router, everywhere);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Find {
    object: WhoHasObject,
    scope: Scope,
    range: Option<RangeInclusive<u32>>,
    wait: Duration,
}

impl Find {
    fn asking(object: WhoHasObject) -> Self {
        Self {
            object,
            scope: Scope::Local,
            range: None,
            wait: FIND_WAIT,
        }
    }

    /// The object whose Object_Name is `name`, compared exactly, on this
    /// network.
    pub fn name(name: impl Into<String>) -> Self {
        Self::asking(WhoHasObject::Name(name.into()))
    }

    /// The object `object`, on this network.
    pub fn object(object: ObjectIdentifier) -> Self {
        Self::asking(WhoHasObject::Identifier(object))
    }

    /// Send the Who-Has to `scope` instead of this network alone: one
    /// remote network, every network, or one station on this network.
    #[must_use]
    pub fn within(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Ask only the devices whose instance is in `instances` (every device
    /// by default). Checked when the search runs: an empty range, or one
    /// past instance 4194303, fails it before anything is sent.
    #[must_use]
    pub fn range(mut self, instances: RangeInclusive<u32>) -> Self {
        self.range = Some(instances);
        self
    }

    /// Listen for answers for `wait` ([`FIND_WAIT`] by default).
    #[must_use]
    pub fn wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Where the Who-Has goes.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

/// One object that answered a [`Client::find`]: what an I-Have said, and
/// where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundObject {
    /// The instance of the device that has the object.
    pub device: u32,
    /// The object.
    pub object: ObjectIdentifier,
    /// The object's name.
    pub name: String,
    /// Where the device answers: the station the I-Have came from, or for
    /// one a router passed on, its station on the network it came from.
    pub address: DeviceAddress,
}

/// Where an I-Have came from, as the discovery table keys a device: on
/// this client's own network (`own`, once known) the local station.
/// `None` when its source names no one station.
fn source(reply: &ReceivedIHave, own: Option<u16>) -> Option<DeviceAddress> {
    let address = match &reply.source_network {
        Some(routed) => DeviceAddress::new(routed.network, &routed.mac_address),
        None => DeviceAddress::local(&reply.source_mac),
    };
    address.ok().map(|address| address.localized(own))
}

impl Scope {
    /// Send one Who-Has for `object` in `range` to this scope, once checked
    /// ([`station`](Self::station)).
    pub(crate) async fn send_who_has<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        station: Option<&Station>,
        object: &WhoHasObject,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        let object = object.clone();
        match (self, station) {
            (Self::Local, _) => client
                .who_has(Broadcast::Local, object, range)
                .await
                .map_err(|error| super::discover::to_local_broadcast(client, error)),
            (Self::Network(network), _) => {
                client
                    .who_has(Broadcast::Network(*network), object, range)
                    .await
            }
            (Self::Directed(_), Some(station)) => {
                client.who_has_directed(station, object, range).await
            }
            (Self::Directed(_), None) => unreachable!("a directed Who-Has has its station"),
            (Self::Global, _) => client.who_has(Broadcast::Global, object, range).await,
        }
    }
}

impl Scope {
    /// Whether `reply` came from where this scope's Who-Has went (#2596),
    /// `own` being the client's network number once known: every answer
    /// for a global one; for a local one, an answer from this network (no
    /// SNET, or SNET `own`); for a remote network, an answer from it (SNET
    /// that network, or none when it is `own`); for a directed one, an
    /// answer from that station. An I-Have from elsewhere during the wait,
    /// unasked or another's answer, is no answer to this search.
    fn answered_by(&self, reply: &ReceivedIHave, own: Option<u16>) -> bool {
        // One rule for a Who-Is's answers and a Who-Has's: where the
        // reply came from, as the discovery table keys a device.
        match source(reply, own) {
            Some(from) => self.covers(&from, own),
            // A source that names no one station is in no scope but all.
            None => matches!(self, Self::Global),
        }
    }
}

/// The key two answers from one station share: the device, the object,
/// and the station, localized with `own`.
type StationKey = (ObjectIdentifier, ObjectIdentifier, Option<DeviceAddress>);

fn station_key(reply: &ReceivedIHave, own: Option<u16>) -> StationKey {
    (
        reply.i_have.device_identifier,
        reply.i_have.object_identifier,
        source(reply, own),
    )
}

/// Send one Who-Has for `object` in `range` to `scope` over `client`, once
/// [`station`] has checked the scope, then collect for `wait` the I-Haves
/// that answer it from that scope ([`Scope::answered_by`]): one per device,
/// object and station, at most
/// [`MAX_I_HAVE_REPLIES`](bacnet_client::client::MAX_I_HAVE_REPLIES) ([`collect_answers`]).
/// The low-level half of [`Client::find`] and
/// [`raw::who_has_replies`](crate::raw::who_has_replies).
///
/// A station is localized with the client's network number as it stands
/// when each answer comes, so its routed copy and its local answer count
/// once against the cap; the list is folded again with the number known
/// at the end, for a number learned between the two.
pub(crate) async fn replies<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    station: Option<&Station>,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
    wait: Duration,
) -> Result<Vec<ReceivedIHave>, Error> {
    // Subscribed first, so no answer can arrive before the receiver.
    let mut rx = client.i_have_events();
    scope.send_who_has(client, station, object, range).await?;
    let deadline = tokio::time::Instant::now() + wait;
    // A station's routed copy is its local answer once the client knows
    // its own network's number (#2572): one answer, so it counts once
    // against the cap.
    let mut stations = HashSet::new();
    let keep = |reply: &ReceivedIHave| {
        let own = client.network_number();
        scope.answered_by(reply, own) && stations.insert(station_key(reply, own))
    };
    let mut heard = collect_answers(&mut rx, deadline, object, range, keep).await;
    // A routed copy heard before the number was learned, and the local
    // answer after, are one answer too.
    let own = client.network_number();
    let mut stations = HashSet::new();
    heard.retain(|reply| stations.insert(station_key(reply, own)));
    Ok(heard)
}

/// Check `scope` for a Who-Has before anything is sent: the station's MAC
/// for a directed one.
pub(crate) fn station<'a, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &'a Scope,
) -> Result<Option<&'a Station>, OpsError> {
    scope.station(client, Ask::WhoHas)
}

impl<T: TransportPort + 'static> Client<T> {
    /// Send the one Who-Has `request` describes, under the rate cap, and
    /// collect for its wait the objects that answer from the scope asked:
    /// one [`FoundObject`] per device address and object, in the order the
    /// I-Haves came. An I-Have from elsewhere during the wait is no answer.
    /// With no scope named the Who-Has is a local broadcast, which no router
    /// passes on, so a device behind a router doesn't hear it: an empty
    /// list says only that nothing on the scope asked has the object.
    /// Collection stops at
    /// [`MAX_I_HAVE_REPLIES`](bacnet_client::client::MAX_I_HAVE_REPLIES)
    /// answers, so a list of exactly that many may have been cut short: ask
    /// a narrower scope or range.
    ///
    /// ```no_run
    /// use bacnet_ops::{Client, Find, Scope};
    /// # async fn example(client: Client) -> Result<(), bacnet_ops::OpsError> {
    /// let mut found = client.find(Find::name("Zone Temp")).await?;
    /// if found.is_empty() {
    ///     found = client.find(Find::name("Zone Temp").within(Scope::Global)).await?;
    /// }
    /// for object in found {
    ///     println!("device {} at {} has {}", object.device, object.address, object.name);
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// Before anything is sent, [`OpsError::Parse`] for an instance range
    /// that is empty or past instance 4194303, or a scope that can't be
    /// sent (see [`Scope`]); then [`OpsError::Client`] for a send the link
    /// refused, when nothing is collected.
    pub async fn find(&self, request: Find) -> Result<Vec<FoundObject>, OpsError> {
        let range = instances(request.range.as_ref())?;
        let client = self.low_level();
        let station = station(client, &request.scope)?;
        // The network the Who-Has goes out on takes its slot, as a
        // Who-Is's does; a global one takes the global slot alone.
        self.limits()
            .slot(request.scope.network(client.network_number()))
            .await;
        let heard = replies(
            client,
            &request.scope,
            station,
            &request.object,
            range,
            request.wait,
        )
        .await;
        // An unconfirmed send that fails never left: the caller isn't
        // charged for it.
        if heard.is_err() {
            crate::limits::refund();
        }
        // Mapped with the number as known now, and folded on it: a station
        // localized differently while the answers came is one row.
        let own = client.network_number();
        let mut rows = HashSet::new();
        Ok(heard
            .map_err(OpsError::from)?
            .into_iter()
            .filter_map(|reply| {
                Some(FoundObject {
                    address: source(&reply, own)?,
                    device: reply.i_have.device_identifier.instance_number(),
                    object: reply.i_have.object_identifier,
                    name: reply.i_have.object_name,
                })
            })
            .filter(|found| rows.insert((found.device, found.object, found.address.clone())))
            .collect())
    }
}
