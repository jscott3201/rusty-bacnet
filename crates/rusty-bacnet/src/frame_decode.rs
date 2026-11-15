//! Python access to the shared, offline Rust frame-summary decoder.

use bacnet_ops::decode::frame::{FrameSummary, Framing, summarize_frame};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// A frozen header summary; unavailable fields remain `None`.
#[pyclass(name = "FrameSummary", module = "rusty_bacnet", frozen)]
pub(crate) struct PyFrameSummary(FrameSummary);

#[pymethods]
impl PyFrameSummary {
    /// Link message name, such as `original-unicast-npdu` or `token`.
    #[getter]
    fn link(&self) -> &str {
        &self.0.link
    }
    /// Whether the headers identify a link or network broadcast.
    #[getter]
    fn broadcast(&self) -> bool {
        self.0.broadcast
    }
    /// Source VMAC or MS/TP station encoded by the link header, if any.
    #[getter]
    fn link_source(&self) -> Option<&str> {
        self.0.link_source.as_deref()
    }
    /// Destination VMAC or MS/TP station encoded by the link header, if any.
    #[getter]
    fn link_destination(&self) -> Option<&str> {
        self.0.link_destination.as_deref()
    }
    /// Original IP address and UDP port encoded by a forwarded message.
    /// The physical forwarding peer comes from `Frame.peer`, not the bytes.
    #[getter]
    fn original_source(&self) -> Option<&str> {
        self.0.original_source.as_deref()
    }
    /// Routed source network number, if present.
    #[getter]
    fn snet(&self) -> Option<u16> {
        self.0.snet
    }
    /// Destination network number, if present.
    #[getter]
    fn dnet(&self) -> Option<u16> {
        self.0.dnet
    }
    /// Routed source in target notation, such as `2:0x05`.
    #[getter]
    fn routed_source(&self) -> Option<&str> {
        self.0.routed_source.as_deref()
    }
    /// Routed destination in target notation, or `2:*` for a broadcast.
    #[getter]
    fn routed_destination(&self) -> Option<&str> {
        self.0.routed_destination.as_deref()
    }
    /// Network-layer message name, if this is a network message.
    #[getter]
    fn network_message(&self) -> Option<&str> {
        self.0.network_message.as_deref()
    }
    /// Application PDU type, such as `Confirmed-Request`.
    #[getter]
    fn pdu(&self) -> Option<&str> {
        self.0.pdu.as_deref()
    }
    /// Application invoke identifier, when the PDU carries one.
    #[getter]
    fn invoke_id(&self) -> Option<u8> {
        self.0.invoke_id
    }
    /// Service name in kebab case, such as `read-property`.
    #[getter]
    fn service(&self) -> Option<&str> {
        self.0.service.as_deref()
    }
    /// Protocol refusal or malformed-header diagnostic; not a stable code.
    #[getter]
    fn problem(&self) -> Option<&str> {
        self.0.problem.as_deref()
    }
    /// Header decode failure only. A valid Error, Reject, Abort or NAK
    /// has a `problem` but no `parse_error`. Service payloads are not fully validated.
    #[getter]
    fn parse_error(&self) -> Option<&str> {
        self.0.parse_error.as_deref()
    }
    /// Device instance announced by an I-Am that could be decoded.
    #[getter]
    fn device(&self) -> Option<u32> {
        self.0.device
    }
}

fn framing(link: &str) -> PyResult<Framing> {
    match link.to_ascii_lowercase().as_str() {
        "bip" | "bacnet/ip" => Ok(Framing::Bip),
        "bip6" | "ipv6" | "bacnet/ipv6" => Ok(Framing::Bip6),
        "sc" | "bacnet/sc" => Ok(Framing::Sc),
        "mstp" | "ms/tp" => Ok(Framing::Mstp),
        _ => Err(PyValueError::new_err(
            "link must be bip, bip6, sc or mstp, or the corresponding Frame.link name; Ethernet is not supported",
        )),
    }
}

/// Decode link, NPDU and APDU headers without sending traffic. Malformed
/// octets return a partial summary with `parse_error`, not a Python parser.
/// `link=frame.link` accepts the observing iterator's link names.
#[pyfunction]
#[pyo3(signature = (octets, link="bip"))]
fn decode_frame(octets: &[u8], link: &str) -> PyResult<PyFrameSummary> {
    Ok(PyFrameSummary(summarize_frame(framing(link)?, octets)))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyFrameSummary>()?;
    m.add_function(wrap_pyfunction!(decode_frame, m)?)?;
    Ok(())
}
