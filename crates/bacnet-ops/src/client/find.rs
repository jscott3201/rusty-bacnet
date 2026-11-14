//! Finding objects through the client (#2596): one Who-Has ([`Find`]) to
//! a [`Scope`], this network alone unless the request names another, and
//! the I-Haves that answer it ([`FoundObject`]).

use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, ReceivedIHave};
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::sync::broadcast::{self, error::RecvError};

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
        station: Option<&[u8]>,
        object: &WhoHasObject,
        range: Option<DeviceInstanceRange>,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        WhoHasRequest {
            range,
            object: object.clone(),
        }
        .encode(&mut buf)?;
        let service = UnconfirmedServiceChoice::WHO_HAS;
        match (self, station) {
            (Self::Local, _) => client.broadcast_unconfirmed(service, &buf).await,
            (Self::Network(network), _) => {
                client
                    .broadcast_network_unconfirmed(service, &buf, *network)
                    .await
            }
            (Self::Directed(_), Some(mac)) => client.unconfirmed_request(mac, service, &buf).await,
            (Self::Directed(_), None) => unreachable!("a directed Who-Has has its station"),
            (Self::Global, _) => client.broadcast_global_unconfirmed(service, &buf).await,
        }
    }
}

/// Collect from `rx` until `deadline` the I-Haves that answer a Who-Has
/// for `object` in `range` ([`ReceivedIHave::answers`]): one per device,
/// object and source, in the order they came. A receiver that falls
/// behind loses the oldest, and collection goes on.
async fn collect(
    rx: &mut broadcast::Receiver<ReceivedIHave>,
    deadline: tokio::time::Instant,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
) -> Vec<ReceivedIHave> {
    let mut replies: Vec<ReceivedIHave> = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(reply)) => {
                let seen = replies.iter().any(|kept| {
                    kept.i_have.device_identifier == reply.i_have.device_identifier
                        && kept.i_have.object_identifier == reply.i_have.object_identifier
                        && kept.source_mac == reply.source_mac
                        && kept.source_network == reply.source_network
                });
                if !seen && reply.answers(object, range) {
                    replies.push(reply);
                }
            }
            Ok(Err(RecvError::Lagged(missed))) => {
                tracing::warn!(missed, "Who-Has collector fell behind; I-Haves were lost");
            }
            Ok(Err(RecvError::Closed)) | Err(_) => return replies,
        }
    }
}

/// Send one Who-Has for `object` in `range` to `scope` over `client`, once
/// [`station`] has checked the scope, then collect the I-Haves that answer
/// it for `wait`: the low-level half of [`Client::find`] and
/// [`raw::who_has_replies`](crate::raw::who_has_replies).
pub(crate) async fn replies<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
    station: Option<&[u8]>,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
    wait: Duration,
) -> Result<Vec<ReceivedIHave>, Error> {
    // Subscribed first, so no answer can arrive before the receiver.
    let mut rx = client.i_have_events();
    scope.send_who_has(client, station, object, range).await?;
    let deadline = tokio::time::Instant::now() + wait;
    Ok(collect(&mut rx, deadline, object, range).await)
}

/// Check `scope` for a Who-Has before anything is sent: the station's MAC
/// for a directed one.
pub(crate) fn station<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    scope: &Scope,
) -> Result<Option<Vec<u8>>, OpsError> {
    scope.station(client, Ask::WhoHas)
}

impl<T: TransportPort + 'static> Client<T> {
    /// Send the one Who-Has `request` describes, under the rate cap, and
    /// collect for its wait the objects that answer: one [`FoundObject`]
    /// per device address and object, in the order the I-Haves came. With
    /// no scope named the Who-Has is a local broadcast, which no router
    /// passes on, so a device behind a router doesn't hear it: an empty
    /// list says only that nothing on the scope asked has the object.
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
        self.limits().slot(request.scope.network()).await;
        let heard = replies(
            client,
            &request.scope,
            station.as_deref(),
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
        let own = client.network_number();
        let mut found: Vec<FoundObject> = Vec::new();
        for reply in heard.map_err(OpsError::from)? {
            let Some(address) = source(&reply, own) else {
                continue;
            };
            let object = FoundObject {
                device: reply.i_have.device_identifier.instance_number(),
                object: reply.i_have.object_identifier,
                name: reply.i_have.object_name,
                address,
            };
            // A routed answer and a local one may name the same station
            // once the network's number is known.
            if !found.contains(&object) {
                found.push(object);
            }
        }
        Ok(found)
    }
}
