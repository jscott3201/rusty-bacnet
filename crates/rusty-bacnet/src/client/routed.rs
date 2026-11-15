//! A `RoutedTarget` as the target of a typed call (#2617): `read_property`,
//! `read_property_multiple`, `read_range`, `read_log_page` and
//! `write_property` take one where they take an address, on a standalone
//! client and on one from `endpoint.client()` alike, as the narrow endpoint
//! client did (#2612). Each call goes through the client's own routed
//! requests, so it is paced, sized and segmented as every other request is.
//!
//! A `str` stays an address on the local link and is never read as a
//! routed one. With `router` given, the request goes through that router.
//! With `router=None` the client uses the route it knows for the station,
//! the router its discovery-table row names, and asks the station's network
//! with a Who-Is first when it has no row, as `device()` does; a station
//! that doesn't answer raises `BacnetLookupError`, which says to pass
//! `router=`, and the client doesn't ask for it again for [`MISS_MEMORY`]
//! unless an I-Am from it arrives. A station on this client's own network,
//! once it knows the number, is sent to directly either way.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_ops::{DeviceAddress, OpsError};
use bacnet_services::read_range::ReadRangeValidation;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeReply, ReadRangeRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;

use super::CoreClient;
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
        }))
    }

    /// The MAC of a local address, parsed for this client's link.
    pub(super) fn local(text: &str, peers: PeerAddresses) -> PyResult<Vec<u8>> {
        peers.parse(text)
    }
}

/// A station on another network, and the router to reach it by, if named.
pub(super) struct Routed {
    network: u16,
    station: Vec<u8>,
    router: Option<Vec<u8>>,
}

/// How one routed call reaches its station.
pub(super) enum Path {
    /// Directly: the station is on this client's own network.
    Local(Vec<u8>),
    /// Through the router at `router` on the local link.
    Via {
        router: Vec<u8>,
        network: u16,
        station: Vec<u8>,
    },
}

impl Routed {
    /// The path to the station on `client`'s link. A named router that is
    /// a broadcast or group address on the local link raises BacnetError:
    /// a routed request goes to its router by unicast. With no router, the
    /// discovery table's row for the station gives it, after one Who-Is to
    /// the station's network when there is no row (`device()`'s rule);
    /// with none after that, BacnetLookupError, and `misses` keeps the
    /// station so a call soon after raises at once instead of asking again.
    pub(super) async fn path(self, client: &OpsClient, misses: &RecentMisses) -> PyResult<Path> {
        let core = client.low_level();
        if let Some(router) = self.router {
            if core.transport().is_group_destination(&router) {
                return Err(to_py_err(Error::Encoding(
                    "a routed request goes to its router by unicast, and this router is a \
                     broadcast or group address on the local link: pass router=None to use \
                     the route the client learns"
                        .into(),
                )));
            }
            return Ok(Path::Via {
                router,
                network: self.network,
                station: self.station,
            });
        }
        let address =
            DeviceAddress::new(self.network, &self.station).map_err(PyValueError::new_err)?;
        let style = client.address_style();
        let at = address.clone().localized(core.network_number());
        let key = (self.network, self.station.clone());
        // A row, from a Who-Is or an I-Am the station sent on its own, is the
        // route at once, whatever an earlier miss said.
        if !at.is_local() && at.row(core).await.is_none() {
            if misses.recent(&key, Instant::now()) {
                return Err(no_route(&at, self.network, Asked::Recently));
            }
            // Asks the network with a Who-Is, as device() does.
            match client.device(address.clone()).await {
                Ok(device) => {
                    drop(device);
                    misses.forget(&key);
                }
                Err(OpsError::UnknownDevice { asked, .. }) => {
                    let asked = if asked.is_some() {
                        misses.remember(key, Instant::now());
                        Asked::Now
                    } else {
                        Asked::Never
                    };
                    return Err(no_route(&at, self.network, asked));
                }
                Err(error) => return Err(crate::high_level::ops_err(error, style)),
            }
        } else if at.is_local() {
            // Checks the MAC against the link and sends nothing.
            drop(
                client
                    .device(address.clone())
                    .await
                    .map_err(|error| crate::high_level::ops_err(error, style))?,
            );
            return Ok(Path::Local(self.station));
        }
        let row = at.row(core).await.ok_or_else(|| {
            crate::errors::lookup_error(
                format!("no route to {at}: the station's row left the discovery table"),
                crate::errors::Lookup::unknown(at.to_string()),
            )
        })?;
        Ok(Path::Via {
            router: row.mac_address.to_vec(),
            network: self.network,
            station: self.station,
        })
    }
}

/// How long a station that didn't answer the client's Who-Is is remembered:
/// a call to it with no router in that time raises at once, unless an I-Am
/// from it has given the client a row.
pub(crate) const MISS_MEMORY: Duration = Duration::from_secs(30);

/// The most stations remembered at once; past it the oldest is forgotten.
const MISS_ROWS: usize = 256;

/// Stations behind a router, by network and MAC, that didn't answer the
/// client's Who-Is, and when (#2617). A std mutex, held only to look up,
/// add or remove.
#[derive(Default)]
pub(super) struct RecentMisses(Mutex<HashMap<(u16, Vec<u8>), Instant>>);

impl RecentMisses {
    /// Whether `station` missed within [`MISS_MEMORY`] of `now`.
    fn recent(&self, station: &(u16, Vec<u8>), now: Instant) -> bool {
        let misses = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        misses
            .get(station)
            .is_some_and(|&missed| now.saturating_duration_since(missed) < MISS_MEMORY)
    }

    /// Keep `station` as missed at `now`, dropping misses past
    /// [`MISS_MEMORY`] and, at [`MISS_ROWS`], the oldest.
    fn remember(&self, station: (u16, Vec<u8>), now: Instant) {
        let mut misses = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        misses.retain(|_, &mut missed| now.saturating_duration_since(missed) < MISS_MEMORY);
        if misses.len() >= MISS_ROWS
            && let Some(oldest) = misses
                .iter()
                .min_by_key(|(_, missed)| **missed)
                .map(|(key, _)| key.clone())
        {
            misses.remove(&oldest);
        }
        misses.insert(station, now);
    }

    /// Forget `station`: it answered.
    fn forget(&self, station: &(u16, Vec<u8>)) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(station);
    }
}

/// Whether the client asked the station's network for it.
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
            "it didn't answer a Who-Is on network {network}, and the client won't ask that \
             network for it again for {seconds} s unless an I-Am from it arrives"
        ),
        Asked::Recently => format!(
            "it didn't answer a Who-Is on network {network} in the last {seconds} s, and no \
             I-Am from it has arrived since"
        ),
        Asked::Never => {
            format!("this client asks no network for a station, network {network} included")
        }
    };
    crate::errors::lookup_error(
        format!(
            "no route to station {at}: {why}. A station that never sends an I-Am, such as an \
             MS/TP slave, needs its router named: RoutedTarget(..., router=...), with the \
             routers on this link from discover_routers()"
        ),
        crate::errors::Lookup::unknown(at.to_string()),
    )
}

/// One confirmed request on `path`, its answer's service data returned.
pub(super) async fn confirmed(
    core: &CoreClient,
    path: &Path,
    service: ConfirmedServiceChoice,
    data: &[u8],
) -> Result<Bytes, Error> {
    match path {
        Path::Local(mac) => core.confirmed_request(mac, service, data).await,
        Path::Via {
            router,
            network,
            station,
        } => {
            core.confirmed_request_routed(router, *network, station, service, data)
                .await
        }
    }
}

/// One ReadRange on `path`, checked against `request` under `validation`,
/// as `read_range_with` checks a local one.
pub(super) async fn read_range(
    core: &CoreClient,
    path: &Path,
    request: &ReadRangeRequest,
    validation: ReadRangeValidation,
) -> Result<ReadRangeReply, Error> {
    let mut buf = bytes::BytesMut::new();
    request.encode(&mut buf)?;
    let answer = confirmed(core, path, ConfirmedServiceChoice::READ_RANGE, &buf).await?;
    ReadRangeReply::check(request, ReadRangeAck::decode(&answer)?, validation)
}

/// The log reader's requests to one station behind a router: each read
/// and range goes to `station` on `network` through the router the reader
/// names, which is the path's.
pub(super) struct RoutedLog<'a> {
    pub(super) core: &'a CoreClient,
    pub(super) network: u16,
    pub(super) station: &'a [u8],
}

impl LogRequester for RoutedLog<'_> {
    async fn read_unsigned(
        &self,
        router: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let ack = self
            .core
            .read_property_routed(router, self.network, self.station, log, property, None)
            .await?;
        unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        router: &[u8],
        request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        let path = Path::Via {
            router: router.to_vec(),
            network: self.network,
            station: self.station.to_vec(),
        };
        read_range(self.core, &path, request, ReadRangeValidation::Lenient).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A miss is kept for [`MISS_MEMORY`], forgotten when the station
    /// answers, and at [`MISS_ROWS`] the oldest goes first.
    #[test]
    fn misses_are_kept_for_a_bounded_time_and_count() {
        let misses = RecentMisses::default();
        let start = Instant::now();
        let station = (5, vec![0x0a]);
        assert!(!misses.recent(&station, start));
        misses.remember(station.clone(), start);
        assert!(misses.recent(&station, start + MISS_MEMORY - Duration::from_millis(1)));
        assert!(!misses.recent(&station, start + MISS_MEMORY));
        misses.forget(&station);
        assert!(!misses.recent(&station, start));

        for octet in 0..MISS_ROWS {
            let later = start + Duration::from_millis(octet as u64);
            misses.remember((7, (octet as u16).to_be_bytes().to_vec()), later);
        }
        let newest = start + Duration::from_secs(1);
        misses.remember((7, vec![0xff, 0xff, 0xff]), newest);
        assert!(!misses.recent(&(7, vec![0, 0]), newest), "the oldest went");
        assert!(misses.recent(&(7, vec![0, 1]), newest));
        assert!(misses.recent(&(7, vec![0xff, 0xff, 0xff]), newest));
        // Expired misses go before the oldest live one.
        misses.remember((8, vec![1]), start + MISS_MEMORY + Duration::from_secs(2));
        assert_eq!(misses.0.lock().unwrap().len(), 1);
    }
}
