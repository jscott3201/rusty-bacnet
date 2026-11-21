//! One record per I-Am received (#2612): `bacnet_ops::arrival` over the
//! owner's frame tap, delivered by the frame iterator's machinery
//! (frames.rs), so it closes, cancels and counts its losses the same way.

use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use bacnet_ops::arrival::{IAmArrival, IAmArrivals};
use bacnet_ops::target::AddressStyle;
use bacnet_transport::tap::FrameTapReceiver;
use bacnet_types::enums::Segmentation;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::frames::{
    Clock, FrameSubscriptions, Gaps, Subscription, TapStream, link_token, peer_text, posix_seconds,
};
use crate::types::{PyObjectIdentifier, PySegmentation};

/// The counts the iterator reads without the stream's lock, still there
/// once the subscription is closed.
#[derive(Default)]
struct Counts {
    malformed: AtomicU64,
    own_echoes: AtomicU64,
}

/// The arrivals on one subscription, publishing their counts.
pub(crate) struct ArrivalStream {
    arrivals: IAmArrivals,
    counts: Arc<Counts>,
}

impl TapStream for ArrivalStream {
    type Item = IAmArrival;

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<IAmArrival>> {
        let polled = self.arrivals.poll_recv(cx);
        let counts = &self.counts;
        counts
            .malformed
            .store(self.arrivals.malformed(), Ordering::Relaxed);
        counts
            .own_echoes
            .store(self.arrivals.own_echoes(), Ordering::Relaxed);
        polled
    }

    fn dropped(&self) -> u64 {
        self.arrivals.missed()
    }

    fn close(self) -> u64 {
        self.arrivals.close()
    }
}

/// Seconds that compare and hash by their bits, so a record is `Eq`.
#[derive(Clone, Copy, Debug)]
struct Seconds(f64);

impl PartialEq for Seconds {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Seconds {}

impl Hash for Seconds {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// One I-Am received, frozen: its fields as Python reads them, so it
/// compares, hashes, copies and pickles by them.
#[pyclass(name = "IAmArrival", module = "rusty_bacnet", frozen, eq, hash)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct PyIAmArrival {
    monotonic: Seconds,
    timestamp: Seconds,
    link: String,
    object_identifier: ObjectIdentifier,
    max_apdu_length: u32,
    segmentation: u8,
    vendor_id: u16,
    link_sender: String,
    link_source: String,
    original_source: Option<String>,
    forwarded: bool,
    source_network: Option<u16>,
    source_address: Option<Vec<u8>>,
    destination_network: Option<u16>,
    destination_address: Option<Vec<u8>>,
    broadcast: bool,
    device_address: Option<String>,
    missed_before: u64,
}

impl PyIAmArrival {
    /// `arrival` for Python, with the gap in front of it as the iterator
    /// counts it.
    fn new(arrival: IAmArrival, missed_before: u64, clock: Clock) -> Self {
        let style = AddressStyle::of_link(arrival.link);
        Self {
            monotonic: Seconds(clock.seconds(arrival.monotonic_timestamp)),
            timestamp: Seconds(posix_seconds(arrival.timestamp)),
            link: link_token(arrival.link).into(),
            object_identifier: arrival.i_am.object_identifier,
            max_apdu_length: arrival.i_am.max_apdu_length,
            segmentation: arrival.i_am.segmentation_supported.to_raw(),
            vendor_id: arrival.i_am.vendor_id,
            link_sender: peer_text(arrival.link_sender),
            link_source: style.format(&arrival.link_source),
            original_source: arrival.original_source.map(|origin| origin.to_string()),
            forwarded: arrival.forwarded,
            source_network: arrival.source_network.as_ref().map(|s| s.network),
            source_address: arrival
                .source_network
                .as_ref()
                .map(|s| s.mac_address.to_vec()),
            destination_network: arrival.destination.as_ref().map(|d| d.network),
            destination_address: arrival.destination.as_ref().map(|d| d.mac_address.to_vec()),
            broadcast: arrival.broadcast,
            device_address: arrival.device_address().map(|a| a.styled(style)),
            missed_before,
        }
    }
}

#[pymethods]
impl PyIAmArrival {
    /// Arrival time on `time.monotonic()`'s clock, as `Frame.monotonic`.
    #[getter]
    fn monotonic(&self) -> f64 {
        self.monotonic.0
    }
    /// Arrival time as POSIX seconds, from the wall clock.
    #[getter]
    fn timestamp(&self) -> f64 {
        self.timestamp.0
    }
    /// The link's `transport=` token, as `Frame.link`.
    #[getter]
    fn link(&self) -> &str {
        &self.link
    }
    /// The I-Am's device identifier, of whatever type it was sent with.
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object_identifier)
    }
    /// Max APDU length accepted, as sent.
    #[getter]
    fn max_apdu_length(&self) -> u32 {
        self.max_apdu_length
    }
    /// Segmentation supported, as sent (a value outside the enumeration
    /// included).
    #[getter]
    fn segmentation_supported(&self) -> PySegmentation {
        PySegmentation {
            inner: Segmentation::from_raw(self.segmentation),
        }
    }
    /// Vendor identifier, as sent.
    #[getter]
    fn vendor_id(&self) -> u16 {
        self.vendor_id
    }
    /// Who put the frame on this node's link, as `Frame.peer` writes it.
    #[getter]
    fn link_sender(&self) -> &str {
        &self.link_sender
    }
    /// The source MAC the stack takes, in the address form of the link.
    #[getter]
    fn link_source(&self) -> &str {
        &self.link_source
    }
    /// The originating node's IP address and port a BBMD wrote into a
    /// Forwarded-NPDU.
    #[getter]
    fn original_source(&self) -> Option<&str> {
        self.original_source.as_deref()
    }
    /// Whether a BBMD forwarded it from another origin.
    #[getter]
    fn forwarded(&self) -> bool {
        self.forwarded
    }
    /// SNET, when a router relayed it.
    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.source_network
    }
    /// SADR, when a router relayed it.
    #[getter]
    fn source_address(&self) -> Option<&[u8]> {
        self.source_address.as_deref()
    }
    /// Destination network in the observed NPDU.
    #[getter]
    fn destination_network(&self) -> Option<u16> {
        self.destination_network
    }

    /// Destination station octets, empty for a network broadcast.
    #[getter]
    fn destination_address(&self) -> Option<&[u8]> {
        self.destination_address.as_deref()
    }

    /// A link or network broadcast.
    #[getter]
    fn broadcast(&self) -> bool {
        self.broadcast
    }
    /// The device's address as `BACnetClient.device()` takes it.
    #[getter]
    fn device_address(&self) -> Option<&str> {
        self.device_address.as_deref()
    }
    /// The router that relayed it, when routed: `link_source`.
    #[getter]
    fn next_hop_router(&self) -> Option<&str> {
        self.source_network.map(|_| self.link_source.as_str())
    }
    /// The BBMD that forwarded it, when forwarded: `link_sender`.
    #[getter]
    fn forwarding_bbmd(&self) -> Option<&str> {
        self.forwarded.then_some(self.link_sender.as_str())
    }
    /// Frames lost just before this record, as `Frame.missed_before`.
    #[getter]
    fn missed_before(&self) -> u64 {
        self.missed_before
    }

    /// The instance, where the device answers and who sent the frame.
    fn __repr__(&self) -> String {
        let address = self
            .device_address
            .as_deref()
            .map_or_else(|| "None".to_string(), |address| format!("'{address}'"));
        format!(
            "IAmArrival(instance={}, device_address={address}, link_sender='{}')",
            self.object_identifier.instance_number(),
            self.link_sender
        )
    }

    /// Copy and pickle by value. Append fields so earlier pickles can be read.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let r = slf.get();
        let fields = [
            r.monotonic.0.into_pyobject(py)?.into_any(),
            r.timestamp.0.into_pyobject(py)?.into_any(),
            r.link.clone().into_pyobject(py)?.into_any(),
            r.object_identifier().into_pyobject(py)?.into_any(),
            r.max_apdu_length.into_pyobject(py)?.into_any(),
            r.segmentation_supported().into_pyobject(py)?.into_any(),
            r.vendor_id.into_pyobject(py)?.into_any(),
            r.link_sender.clone().into_pyobject(py)?.into_any(),
            r.link_source.clone().into_pyobject(py)?.into_any(),
            r.original_source.clone().into_pyobject(py)?,
            r.forwarded.into_pyobject(py)?.to_owned().into_any(),
            r.source_network.into_pyobject(py)?,
            r.source_address
                .as_deref()
                .map(|octets| pyo3::types::PyBytes::new(py, octets))
                .into_pyobject(py)?,
            r.broadcast.into_pyobject(py)?.to_owned().into_any(),
            r.device_address.clone().into_pyobject(py)?,
            r.missed_before.into_pyobject(py)?.into_any(),
            r.destination_network.into_pyobject(py)?,
            r.destination_address
                .as_deref()
                .map(|octets| pyo3::types::PyBytes::new(py, octets))
                .into_pyobject(py)?,
        ];
        Ok((
            slf.get_type().getattr("_rebuild")?,
            PyTuple::new(py, fields)?,
        ))
    }

    /// A record from a pickle's fields, as [`Self::__reduce__`] lists
    /// them. Nothing is checked or decoded.
    #[staticmethod]
    #[pyo3(signature = (monotonic, timestamp, link, object_identifier, max_apdu_length, segmentation_supported, vendor_id, link_sender, link_source, original_source, forwarded, source_network, source_address, broadcast, device_address, missed_before, destination_network=None, destination_address=None))]
    fn _rebuild(
        monotonic: f64,
        timestamp: f64,
        link: String,
        object_identifier: PyObjectIdentifier,
        max_apdu_length: u32,
        segmentation_supported: PySegmentation,
        vendor_id: u16,
        link_sender: String,
        link_source: String,
        original_source: Option<String>,
        forwarded: bool,
        source_network: Option<u16>,
        source_address: Option<Vec<u8>>,
        broadcast: bool,
        device_address: Option<String>,
        missed_before: u64,
        destination_network: Option<u16>,
        destination_address: Option<Vec<u8>>,
    ) -> Self {
        Self {
            monotonic: Seconds(monotonic),
            timestamp: Seconds(timestamp),
            link,
            object_identifier: object_identifier.to_rust(),
            max_apdu_length,
            segmentation: segmentation_supported.to_rust().to_raw(),
            vendor_id,
            link_sender,
            link_source,
            original_source,
            forwarded,
            source_network,
            source_address,
            destination_network,
            destination_address,
            broadcast,
            device_address,
            missed_before,
        }
    }
}

/// A bounded subscription to the I-Ams the owner's transport receives.
#[pyclass(name = "IAmArrivalIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyIAmArrivalIterator {
    subscription: Arc<Subscription<ArrivalStream>>,
    counts: Arc<Counts>,
    clock: Clock,
}

/// Subscribe `owner` to the I-Ams on `receiver`. An owner that sends I-Ams
/// passes `own`, its B/IP or B/IPv6 socket and its device instance, to
/// leave out and count its own I-Am come back; a client passes `None`.
pub(crate) fn subscribe(
    owner: &FrameSubscriptions,
    receiver: FrameTapReceiver,
    own: Option<(SocketAddr, u32)>,
    clock: Clock,
) -> PyIAmArrivalIterator {
    let arrivals = IAmArrivals::new(receiver);
    let arrivals = match own {
        Some((address, instance)) => arrivals.with_own(address, instance),
        None => arrivals,
    };
    let counts = Arc::new(Counts::default());
    PyIAmArrivalIterator {
        subscription: owner.open(ArrivalStream {
            arrivals,
            counts: counts.clone(),
        }),
        counts,
        clock,
    }
}

impl PyIAmArrivalIterator {
    /// What a read's future needs to make a Python record; nothing of the
    /// iterator.
    fn converter(
        &self,
    ) -> impl FnOnce(IAmArrival, &Gaps<ArrivalStream>) -> PyIAmArrival + Send + 'static + use<>
    {
        let clock = self.clock;
        move |arrival, gaps| {
            let missed_before = gaps.missed_before(arrival.missed_before);
            PyIAmArrival::new(arrival, missed_before, clock)
        }
    }
}

#[pymethods]
impl PyIAmArrivalIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.subscription
            .anext(py, "I-Am arrival observation", self.converter())
    }

    /// Frames of any kind lost so far, live, as `FrameIterator.missed`.
    /// Read with the GIL released: a read in progress holds the stream.
    #[getter]
    fn missed(&self, py: Python<'_>) -> u64 {
        py.detach(|| self.subscription.missed())
    }

    /// Inbound I-Am frames read so far that made no record.
    #[getter]
    fn malformed(&self) -> u64 {
        self.counts.malformed.load(Ordering::Relaxed)
    }

    /// The owner's own I-Ams come back to it, read so far and left out.
    #[getter]
    fn own_echoes(&self) -> u64 {
        self.counts.own_echoes.load(Ordering::Relaxed)
    }

    /// Stop observing, as `FrameIterator.aclose`.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.subscription.aclose(py)
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyIAmArrival>()?;
    m.add_class::<PyIAmArrivalIterator>()?;
    Ok(())
}

#[cfg(test)]
#[path = "i_am_arrivals_tests.rs"]
mod tests;
