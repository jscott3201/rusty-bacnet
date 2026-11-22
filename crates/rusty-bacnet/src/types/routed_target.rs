//! `RoutedTarget` (#2612): a station on another BACnet network as a
//! request's target. A `str` address names a station on the local link and
//! is never read as anything else (#2561); this names one behind a router.

use pyo3::exceptions::PyTypeError;
use pyo3::types::{PyBool, PyString};

use super::*;

/// A station on another BACnet network: `RoutedTarget(network, address,
/// router=None, *, arcnet=False)`, in that order.
///
/// `network` is the station's network number, an `int` from 1 to 65534
/// (ValueError outside it, OverflowError outside 0 to 65535, TypeError for a
/// `bool`). `address` is the station's MAC on that network, 1 to 18 octets:
/// `bytes` as given, such as a `DiscoveredDevice`'s `source_address`, or a
/// string in the binding's address forms, `"ip:port"` (a B/IP station's MAC
/// is its IPv4 address and UDP port on any B/IP network, local or not),
/// `"bip6:N"` or `"bip6:0x..."` (a B/IPv6 VMAC), two or more hex octets
/// `"aa:bb:..."`, or one octet in decimal as `"N"` or `"mstp:N"` (0 to 254,
/// so `RoutedTarget(5, "10") == RoutedTarget(5, b"\x0a")`). The string
/// forms refuse 255, MS/TP's broadcast. A confirmed call refuses an address
/// that is a broadcast on its network, before anything is sent (#2709):
/// `b"\xff"` (MS/TP's broadcast), three octets of `0xFF` (no device's VMAC)
/// and six (Ethernet's and BACnet/SC's broadcast). `arcnet=True` says the
/// network is ARCNET, whose broadcast is 0 and where 255 is a station: a
/// call then goes to `b"\xff"` and refuses `b"\x00"`; it takes a one-octet
/// address only. `router`, when given, is the next hop's address on the
/// local link, in the same forms or as `bytes`, such as a routed
/// `DiscoveredDevice`'s `mac_address`; a call refuses one that is a
/// broadcast or group address on the local link (#2722). Without an
/// explicit next hop the client reuses its learned route, or learns one
/// from a Who-Is addressed to this station, then one to its network if
/// nothing answers (an ARCNET station 255 only from its row).
/// Invalid addresses raise ValueError, or TypeError for a type that is neither `str`
/// nor `bytes`; a call in another order fails too, since `network` takes an
/// `int`.
///
/// Frozen; compares and hashes by its network, octets and ARCNET mark,
/// however they were written; copies and pickles.
#[pyclass(
    name = "RoutedTarget",
    module = "rusty_bacnet",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PyRoutedTarget {
    network: u16,
    address: Vec<u8>,
    router: Option<Vec<u8>>,
    arcnet: bool,
}

impl PyRoutedTarget {
    pub(crate) fn station(&self) -> bacnet_client::client::Station {
        use bacnet_client::client::Station;
        let (network, mac, router) = self.parts();
        let station = match router {
            Some(router) => Station::routed(router, network, mac),
            None => Station::remote(network, mac),
        };
        if self.arcnet {
            station.arcnet()
        } else {
            station
        }
    }

    /// The station's network, its MAC there, and the router's MAC, if named.
    pub(crate) fn parts(&self) -> (u16, &[u8], Option<&[u8]>) {
        (self.network, &self.address, self.router.as_deref())
    }

    /// Whether the station's network is marked ARCNET.
    pub(crate) fn is_arcnet(&self) -> bool {
        self.arcnet
    }
}

/// The MAC `value` names: a `str` in [`parse_address`]'s forms, or `bytes`;
/// 1 to 18 octets, the most an NPDU's DADR carries.
fn mac(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<u8>> {
    let mac = if let Ok(text) = value.cast::<PyString>() {
        parse_address(&text.to_cow()?)?
    } else if let Ok(octets) = value.cast::<PyBytes>() {
        octets.as_bytes().to_vec()
    } else {
        return Err(PyTypeError::new_err(format!(
            "RoutedTarget {what} must be str or bytes, not {}",
            value.get_type().name()?
        )));
    };
    if !(1..=bacnet_npdu::MAX_MAC_LEN).contains(&mac.len()) {
        return Err(PyValueError::new_err(format!(
            "RoutedTarget {what} must be 1 to {} octets, got {}",
            bacnet_npdu::MAX_MAC_LEN,
            mac.len()
        )));
    }
    Ok(mac)
}

#[pymethods]
impl PyRoutedTarget {
    /// Check and keep the station's network and MAC, the router's MAC, and
    /// the ARCNET mark.
    #[new]
    #[pyo3(signature = (network, address, router=None, *, arcnet=false))]
    fn new(
        network: &Bound<'_, PyAny>,
        address: &Bound<'_, PyAny>,
        router: Option<&Bound<'_, PyAny>>,
        arcnet: bool,
    ) -> PyResult<Self> {
        // `True` is an `int` to Python, but no network number.
        if network.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "RoutedTarget network must be an int, not bool",
            ));
        }
        let network: u16 = network.extract()?;
        // 0 names the local network in an address, 65535 every network.
        if !(1..=0xFFFE).contains(&network) {
            return Err(PyValueError::new_err(format!(
                "RoutedTarget network must be 1 to 65534, got {network}"
            )));
        }
        let address = mac(address, "address")?;
        // ARCNET's MAC is one octet (Clause 8).
        if arcnet && address.len() != 1 {
            return Err(PyValueError::new_err(format!(
                "RoutedTarget arcnet=True takes a one-octet address (an ARCNET station), got {} \
                 octets",
                address.len()
            )));
        }
        Ok(Self {
            network,
            address,
            router: router.map(|router| mac(router, "router")).transpose()?,
            arcnet,
        })
    }

    /// The station's network number.
    #[getter]
    fn network(&self) -> u16 {
        self.network
    }

    /// The station's MAC on its network.
    #[getter]
    fn address(&self) -> Vec<u8> {
        self.address.clone()
    }

    /// The router's MAC on the local link, or None.
    #[getter]
    fn router(&self) -> Option<Vec<u8>> {
        self.router.clone()
    }

    /// Whether the station's network is marked ARCNET: its `b"\xff"` is a
    /// station and `b"\x00"` the broadcast.
    #[getter]
    fn arcnet(&self) -> bool {
        self.arcnet
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let octets = |mac: &[u8]| -> PyResult<String> {
            Ok(PyBytes::new(py, mac).repr()?.to_cow()?.into_owned())
        };
        let router = match &self.router {
            Some(router) => octets(router)?,
            None => "None".to_owned(),
        };
        let arcnet = if self.arcnet { ", arcnet=True" } else { "" };
        Ok(format!(
            "RoutedTarget(network={}, address={}, router={router}{arcnet})",
            self.network,
            octets(&self.address)?
        ))
    }

    /// Copy and pickle as a call to the constructor with the octets, so a
    /// copy is equal and checked again: `RoutedTarget` itself, or with the
    /// ARCNET mark a `functools.partial` of it that passes `arcnet=True`.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, (u16, Vec<u8>, Option<Vec<u8>>))> {
        let this = slf.get();
        let py = slf.py();
        let make = if this.arcnet {
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("arcnet", true)?;
            py.import("functools")?
                .getattr("partial")?
                .call((slf.get_type(),), Some(&kwargs))?
        } else {
            slf.get_type().into_any()
        };
        Ok((
            make,
            (this.network, this.address.clone(), this.router.clone()),
        ))
    }
}
