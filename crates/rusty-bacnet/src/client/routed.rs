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
//! that doesn't answer raises `BacnetLookupError`. A station on this
//! client's own network, once it knows the number, is sent to directly
//! either way.

use bacnet_client::log_reader::{LogRequester, unsigned_value};
use bacnet_ops::DeviceAddress;
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
    /// with none after that, BacnetLookupError.
    pub(super) async fn path(self, client: &OpsClient) -> PyResult<Path> {
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
        // Asks the network with a Who-Is when the table has no row for the
        // station; a station that never answers is a lookup error.
        drop(
            client
                .device(address.clone())
                .await
                .map_err(|error| crate::high_level::ops_err(error, style))?,
        );
        let at = address.localized(core.network_number());
        if at.is_local() {
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
