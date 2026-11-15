//! Python access to the shared, offline Rust frame-summary decoder.

use bacnet_ops::decode::frame::{FrameSummary, Framing, summarize_frame};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// A frozen header summary; unavailable fields remain `None`. Two
/// summaries are equal, and hash alike, when every field is.
#[pyclass(name = "FrameSummary", module = "rusty_bacnet", frozen, eq, hash)]
#[derive(PartialEq, Eq, Hash)]
pub(crate) struct PyFrameSummary(FrameSummary);

#[pymethods]
impl PyFrameSummary {
    /// The link message, then the PDU, service and invoke ID and the parse
    /// failure or problem, where the summary has them.
    fn __repr__(&self) -> String {
        let summary = &self.0;
        let mut parts = vec![format!("link_message={:?}", summary.link_message)];
        if let Some(pdu) = &summary.pdu {
            parts.push(format!("pdu={pdu:?}"));
        }
        if let Some(service) = &summary.service {
            parts.push(format!("service={service:?}"));
        }
        if let Some(id) = summary.invoke_id {
            parts.push(format!("invoke_id={id}"));
        }
        if let Some(error) = &summary.parse_error {
            parts.push(format!("parse_error={error:?}"));
        } else if let Some(problem) = &summary.problem {
            parts.push(format!("problem={problem:?}"));
        }
        format!("FrameSummary({})", parts.join(", "))
    }

    /// The link message, a BVLC function or MS/TP frame type such as
    /// `original-unicast-npdu` or `token`; not the data link (`Frame.link`).
    #[getter]
    fn link_message(&self) -> &str {
        &self.0.link_message
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
    /// The originating node's IP address and UDP port a BBMD wrote into a
    /// B/IP Forwarded-NPDU, or a B/IPv6 Forwarded-NPDU or
    /// Forwarded-Address-Resolution. The peer that sent the frame is
    /// `Frame.peer`; it is not in the octets.
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
/// `link` has no default: octets read as the wrong link come back as a
/// parse failure, never as another link's message. `link=frame.link`
/// accepts the observing iterator's link names.
#[pyfunction]
#[pyo3(signature = (octets, link))]
fn decode_frame(octets: &[u8], link: &str) -> PyResult<PyFrameSummary> {
    Ok(PyFrameSummary(summarize_frame(framing(link)?, octets)))
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyFrameSummary>()?;
    m.add_function(wrap_pyfunction!(decode_frame, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::exceptions::PyTypeError;
    use pyo3::types::PyDict;

    /// Octets of a ReadProperty request on B/IP, invoke ID 1.
    const REQUEST: &[u8] = &[
        0x81, 0x0A, 0x00, 0x11, 0x01, 0x00, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x03, 0x12,
        0x19, 0x4B,
    ];

    fn module(py: Python<'_>) -> Bound<'_, PyModule> {
        let module = PyModule::new(py, "frames").unwrap();
        register(&module).unwrap();
        module
    }

    /// `link` must be given, by position or keyword; there is no default.
    #[test]
    fn decode_frame_needs_its_link() {
        Python::initialize();
        Python::attach(|py| {
            let decode = module(py).getattr("decode_frame").unwrap();
            let error = decode.call1((REQUEST,)).unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py), "{error}");
            let positional = decode.call1((REQUEST, "bip")).unwrap();
            let kwargs = PyDict::new(py);
            kwargs.set_item("link", "bip").unwrap();
            let keyword = decode.call((REQUEST,), Some(&kwargs)).unwrap();
            assert!(positional.eq(&keyword).unwrap());
            let message: String = keyword.getattr("link_message").unwrap().extract().unwrap();
            assert_eq!(message, "original-unicast-npdu");
        });
    }

    /// Summaries compare and hash by value, and print in one short line.
    #[test]
    fn summaries_compare_hash_and_print_by_value() {
        Python::initialize();
        Python::attach(|py| {
            let decode = module(py).getattr("decode_frame").unwrap();
            let one = decode.call1((REQUEST, "bip")).unwrap();
            let again = decode.call1((REQUEST, "bip")).unwrap();
            let other = decode.call1((&REQUEST[..3], "bip")).unwrap();
            assert!(one.eq(&again).unwrap());
            assert!(!one.eq(&other).unwrap());
            assert_eq!(one.hash().unwrap(), again.hash().unwrap());
            assert_eq!(
                one.repr().unwrap().to_string(),
                "FrameSummary(link_message=\"original-unicast-npdu\", pdu=\"Confirmed-Request\", \
                 service=\"read-property\", invoke_id=1)"
            );
            assert!(
                other
                    .repr()
                    .unwrap()
                    .to_string()
                    .starts_with("FrameSummary(link_message=\"malformed\", parse_error=")
            );
        });
    }
}
