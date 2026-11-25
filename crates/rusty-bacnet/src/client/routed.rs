//! A `RoutedTarget` as the target of a typed call (#2617, #2613, #2588):
//! `read_property`, `read_property_multiple`, `read_range`, `read_log_page`,
//! `write_property`, `atomic_read_file`, `atomic_write_file`,
//! `subscribe_cov`, `unsubscribe_cov`, `subscribe_cov_property`,
//! `unsubscribe_cov_property` and `subscribe_cov_property_multiple` take one
//! where they take an address, on a standalone client and on one from
//! `endpoint.client()` alike (#2612). Each call goes through the client's
//! own routed requests, so it is paced, sized and segmented as every other
//! request is.
//!
//! A `str` stays an address on the local link and is never read as a
//! routed one. The client checks each call's station as it checks every
//! confirmed request (#2722, #2709): this binding keeps no copy of those
//! checks. With `router` given, the request goes through that router.
//! With `router=None` the client uses the route it knows for the station,
//! the router its discovery-table row names, and when it has no row finds
//! the station as `device()` does: a Who-Is addressed to the station, then
//! one to its network. A station that answers neither raises
//! `BacnetLookupError`, which says to pass `router=`, and the client doesn't
//! ask for it again for [`MISS_MEMORY`] unless an I-Am from it arrives;
//! bacnet-ops keeps that memory ([`OpsClient::unanswered_recently`]). A
//! station on this client's own network, once it knows the number, is sent
//! to directly either way.

use std::collections::HashMap;
use std::sync::Mutex;

use bacnet_client::client::Station;
use bacnet_ops::device::MISS_MEMORY;
use bacnet_ops::{DeviceAddress, OpsError};
use bacnet_types::sync::MutexExt;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;
use tokio::sync::watch;

use crate::errors::to_py_err;
use crate::high_level::OpsClient;
use crate::types::{PeerAddresses, PyRoutedTarget};

/// What a typed call's `address` names: a station on the local link, as a
/// `str` always has, or a station behind a router.
pub(super) enum Address {
    /// A `str` in the client's address forms, parsed when the call runs.
    Local(String),
    /// A `RoutedTarget`.
    Routed(Routed),
}

impl Address {
    /// `address` as a call's target: a `str` or a `RoutedTarget`, and a
    /// TypeError for anything else, before anything is sent.
    pub(super) fn of(address: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(text) = address.cast::<PyString>() {
            return Ok(Self::Local(text.to_cow()?.into_owned()));
        }
        let Ok(target) = address.cast::<PyRoutedTarget>() else {
            return Err(PyTypeError::new_err(format!(
                "address must be a str or a RoutedTarget, not {}",
                address.get_type().name()?
            )));
        };
        let (network, station, router) = target.get().parts();
        Ok(Self::Routed(Routed {
            network,
            station: station.to_vec(),
            router: router.map(<[u8]>::to_vec),
            one_station: target.get().is_one_station(),
        }))
    }

    /// The MAC of a local address, parsed for this client's link.
    pub(super) fn local(text: &str, peers: PeerAddresses) -> PyResult<Vec<u8>> {
        peers.parse(text)
    }
}

/// A station on another network, the router to reach it by, if named, and
/// whether its address is marked one station.
pub(super) struct Routed {
    network: u16,
    station: Vec<u8>,
    router: Option<Vec<u8>>,
    one_station: bool,
}

impl Routed {
    /// The path to the station on `client`'s link, for a confirmed call.
    /// The client's checks of a confirmed request that need no link come
    /// first ([`Station::check_confirmed`]): a remote broadcast form, as
    /// `one_station` has them (#2709, #2738), raises BacnetError before any
    /// lookup, so it is never asked for or remembered as a miss. A named
    /// router that is a broadcast or group address on the local link is
    /// the client's to refuse, when the call is made (#2722).
    ///
    /// With no router, the discovery table's row for the station gives it,
    /// after a Who-Is to the station when there is no row and, unanswered,
    /// one to its network (`device()`'s rule, up to two resolve waits);
    /// with none after that, BacnetLookupError, and a call soon after
    /// raises at once instead of asking again (bacnet-ops remembers the
    /// miss). `lookups` keeps one lookup per station in flight. A station
    /// marked one station at a form bacnet-ops refuses as an address (the
    /// one octet `0xFF`, two octets starting `0x00` or `0xFF`) has no
    /// address it takes, so with no router it is reached through its row
    /// alone.
    pub(super) async fn path(
        self,
        client: &OpsClient,
        lookups: &StationLookups,
    ) -> PyResult<Station> {
        let named = match &self.router {
            Some(router) => Station::routed(router, self.network, &self.station),
            None => Station::remote(self.network, &self.station),
        };
        let named = if self.one_station {
            named.one_station()
        } else {
            named
        };
        named.check_confirmed().map_err(to_py_err)?;
        if self.router.is_some() {
            return Ok(named);
        }
        let core = client.low_level();
        if self.one_station && bacnet_npdu::RemoteBroadcast::of(&self.station, false).is_some() {
            let shown = format!(
                "{}:0x{}",
                self.network,
                bacnet_ops::octets::hex_upper(&self.station)
            );
            return match core.get_device_at(self.network, &self.station).await {
                Some(row) => Ok(
                    Station::routed(&row.mac_address, self.network, &self.station).one_station(),
                ),
                None => Err(crate::errors::lookup_error(
                    format!(
                        "no route to {shown}, marked one station: the discovery table has no \
                         row for it; name its router: RoutedTarget(..., router=..., \
                         one_station=True)"
                    ),
                    crate::errors::Lookup::unknown(shown),
                )),
            };
        }
        let address =
            DeviceAddress::new(self.network, &self.station).map_err(PyValueError::new_err)?;
        let style = client.address_style();
        let at = address.clone().localized(core.network_number());
        let key = (self.network, self.station.clone());
        // A row, from a Who-Is or an I-Am the station sent on its own, is the
        // route at once, whatever an earlier miss said.
        if !at.is_local() && at.row(core).await.is_none() {
            // One lookup per station at a time: calls that find one in
            // flight wait for what it finds instead of asking again.
            loop {
                if client.unanswered_recently(&at) {
                    return Err(no_route(&at, self.network, Asked::Recently));
                }
                match lookups.turn(&key) {
                    Turn::Wait(mut answer) => {
                        let found = answer.wait_for(Option::is_some).await.map(|found| *found);
                        match found {
                            Ok(Some(Found::Row)) => break,
                            Ok(Some(Found::Missed(asked))) => {
                                return Err(no_route(&at, self.network, asked));
                            }
                            // The call that asked went away before it
                            // knew: ask again.
                            Ok(None) | Err(_) => continue,
                        }
                    }
                    // Asks as device() does: a Who-Is to the station, then,
                    // unanswered, one to its network.
                    Turn::Ask(asking) => match client.device(address.clone()).await {
                        Ok(device) => {
                            drop(device);
                            asking.finish(Found::Row);
                            break;
                        }
                        Err(OpsError::UnknownDevice { asked, .. }) => {
                            // bacnet-ops remembered the miss when it asked.
                            let asked = if asked.is_some() {
                                Asked::Now
                            } else {
                                Asked::Never
                            };
                            asking.finish(Found::Missed(asked));
                            return Err(no_route(&at, self.network, asked));
                        }
                        // The waiters ask again for themselves.
                        Err(error) => return Err(crate::high_level::ops_err(error, style)),
                    },
                }
            }
        } else if at.is_local() {
            // Checks the MAC against the link and sends nothing.
            drop(
                client
                    .device(address.clone())
                    .await
                    .map_err(|error| crate::high_level::ops_err(error, style))?,
            );
            return Ok(Station::local(&self.station));
        }
        let row = at.row(core).await.ok_or_else(|| {
            crate::errors::lookup_error(
                format!("no route to {at}: the station's row left the discovery table"),
                crate::errors::Lookup::unknown(at.to_string()),
            )
        })?;
        let routed = Station::routed(&row.mac_address, self.network, &self.station);
        Ok(if self.one_station {
            routed.one_station()
        } else {
            routed
        })
    }
}

/// A station behind a router: its network and its MAC there.
type StationKey = (u16, Vec<u8>);

/// The lookups in flight for stations behind a router, one per station
/// (#2617): a call that finds one here waits for what it finds. Which
/// stations missed is bacnet-ops' memory
/// ([`OpsClient::unanswered_recently`]). A std mutex, held only to look up,
/// add or remove, never across an await.
#[derive(Default)]
pub(super) struct StationLookups {
    asking: Mutex<HashMap<StationKey, watch::Receiver<Option<Found>>>>,
}

/// What one lookup found, for the calls that waited on it.
#[derive(Clone, Copy)]
enum Found {
    /// A row: the route.
    Row,
    /// No answer.
    Missed(Asked),
}

/// A call's part in a station's lookup.
enum Turn<'a> {
    /// Ask: no lookup is in flight.
    Ask(Asking<'a>),
    /// Wait for the one in flight.
    Wait(watch::Receiver<Option<Found>>),
}

/// The one lookup in flight for a station. Dropped, finished or not, it
/// leaves the map; a call that waited on an unfinished one asks itself.
struct Asking<'a> {
    lookups: &'a StationLookups,
    station: StationKey,
    found: watch::Sender<Option<Found>>,
}

impl Asking<'_> {
    /// Tell the calls waiting what this lookup found.
    fn finish(&self, found: Found) {
        self.found.send_replace(Some(found));
    }
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        // Only this lookup inserts the station's entry, so it is its own.
        self.lookups.asking.locked().remove(&self.station);
    }
}

impl StationLookups {
    /// Join the lookup in flight for `station`, or start it.
    fn turn(&self, station: &StationKey) -> Turn<'_> {
        let mut asking = self.asking.locked();
        if let Some(answer) = asking.get(station) {
            return Turn::Wait(answer.clone());
        }
        let (found, answer) = watch::channel(None);
        asking.insert(station.clone(), answer);
        Turn::Ask(Asking {
            lookups: self,
            station: station.clone(),
            found,
        })
    }
}

/// Whether the client asked the station.
#[derive(Clone, Copy)]
enum Asked {
    /// Just now, with no answer.
    Now,
    /// Within [`MISS_MEMORY`], with no answer: not again yet.
    Recently,
    /// Not at all: this client asks no network (no resolve wait).
    Never,
}

/// The BacnetLookupError for a station at `at` the client has no route to,
/// saying what to do: name its router.
fn no_route(at: &DeviceAddress, network: u16, asked: Asked) -> PyErr {
    let seconds = MISS_MEMORY.as_secs();
    let why = match asked {
        Asked::Now => format!(
            "it answered neither a Who-Is addressed to it nor one to network {network}, and \
             the client won't ask that station again for {seconds} s unless an I-Am from it \
             arrives"
        ),
        Asked::Recently => format!(
            "it answered neither a Who-Is addressed to it nor one to network {network} in \
             the last {seconds} s, and no I-Am from it has arrived since"
        ),
        Asked::Never => {
            format!("this client asks no network for a station, network {network} included")
        }
    };
    crate::errors::lookup_error(
        format!(
            "no route to station {at}: {why}. A station that no I-Am comes for, such as an \
             MS/TP slave with no slave proxy, needs its router named: RoutedTarget(..., router=...), with the \
             routers on this link from discover_routers()"
        ),
        crate::errors::Lookup::unknown(at.to_string()),
    )
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    /// One lookup per station: a second call waits on the first, which
    /// leaves the map when it ends, finished or dropped.
    #[tokio::test]
    async fn one_lookup_per_station_and_a_dropped_one_lets_the_next_ask() {
        let misses = StationLookups::default();
        let station = (5, vec![0x0a]);
        let Turn::Ask(first) = misses.turn(&station) else {
            panic!("the first call asks");
        };
        let Turn::Wait(mut waiting) = misses.turn(&station) else {
            panic!("the second waits");
        };
        assert!(
            matches!(misses.turn(&(6, vec![0x0a])), Turn::Ask(_)),
            "another station asks"
        );
        first.finish(Found::Missed(Asked::Now));
        let found = waiting.wait_for(Option::is_some).await.map(|found| *found);
        assert!(matches!(found, Ok(Some(Found::Missed(Asked::Now)))));
        drop(first);
        // Dropped unfinished, a lookup wakes its waiters with nothing.
        let Turn::Ask(second) = misses.turn(&station) else {
            panic!("a finished lookup left the map");
        };
        let Turn::Wait(mut waiting) = misses.turn(&station) else {
            panic!("waits on the second");
        };
        drop(second);
        assert!(waiting.wait_for(Option::is_some).await.is_err());
        assert!(matches!(misses.turn(&station), Turn::Ask(_)));
    }
}
