//! Python access to the shared, offline Rust frame-summary decoder.

use bacnet_ops::decode::frame::{FrameSummary, Framing, summarize_frame};
use std::hash::{Hash, Hasher};

use bacnet_types::data_link::DataLink;
use pyo3::exceptions::PyValueError;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::frames::link_token;

/// A frozen header summary; unavailable fields remain `None`. Two
/// summaries are equal, and hash alike, when every field is.
#[pyclass(name = "FrameSummary", module = "rusty_bacnet", frozen, eq, hash)]
pub(crate) struct PyFrameSummary {
    summary: FrameSummary,
    /// The octets and `Frame.link` token it was decoded from, which `copy`
    /// and `pickle` decode again; not part of its value.
    octets: Box<[u8]>,
    link: &'static str,
}

/// What [`PyFrameSummary::__reduce__`] gives `copy` and `pickle`:
/// `decode_frame` and its arguments.
type Reduce<'py> = (Bound<'py, PyAny>, (Bound<'py, PyBytes>, &'static str));

impl PartialEq for PyFrameSummary {
    fn eq(&self, other: &Self) -> bool {
        self.summary == other.summary
    }
}

impl Eq for PyFrameSummary {}

impl Hash for PyFrameSummary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.summary.hash(state);
    }
}

#[pymethods]
impl PyFrameSummary {
    /// The link message, then the PDU, service and invoke ID and the parse
    /// failure or problem, where the summary has them.
    fn __repr__(&self) -> String {
        let summary = &self.summary;
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

    /// `copy` and `pickle` call `decode_frame` on the same octets and link
    /// again, which gives an equal summary.
    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> PyResult<Reduce<'py>> {
        let py = slf.py();
        let this = slf.get();
        let decode = py
            .import(intern!(py, "rusty_bacnet"))?
            .getattr(intern!(py, "decode_frame"))?;
        Ok((decode, (PyBytes::new(py, &this.octets), this.link)))
    }

    /// The link message, a BVLC function or MS/TP frame type such as
    /// `original-unicast-npdu` or `token`; not the data link (`Frame.link`).
    #[getter]
    fn link_message(&self) -> &str {
        &self.summary.link_message
    }
    /// Whether the headers identify a link or network broadcast.
    #[getter]
    fn broadcast(&self) -> bool {
        self.summary.broadcast
    }
    /// Source VMAC or MS/TP station encoded by the link header, if any.
    #[getter]
    fn link_source(&self) -> Option<&str> {
        self.summary.link_source.as_deref()
    }
    /// Destination VMAC or MS/TP station encoded by the link header, if any.
    #[getter]
    fn link_destination(&self) -> Option<&str> {
        self.summary.link_destination.as_deref()
    }
    /// The originating node's IP address and UDP port a BBMD wrote into a
    /// B/IP Forwarded-NPDU, or a B/IPv6 Forwarded-NPDU or
    /// Forwarded-Address-Resolution. The peer that sent the frame is
    /// `Frame.peer`; it is not in the octets.
    #[getter]
    fn original_source(&self) -> Option<&str> {
        self.summary.original_source.as_deref()
    }
    /// Routed source network number, if present.
    #[getter]
    fn snet(&self) -> Option<u16> {
        self.summary.snet
    }
    /// Destination network number, if present.
    #[getter]
    fn dnet(&self) -> Option<u16> {
        self.summary.dnet
    }
    /// Routed source in target notation, such as `2:0x05`.
    #[getter]
    fn routed_source(&self) -> Option<&str> {
        self.summary.routed_source.as_deref()
    }
    /// Routed destination in target notation, or `2:*` for a broadcast.
    #[getter]
    fn routed_destination(&self) -> Option<&str> {
        self.summary.routed_destination.as_deref()
    }
    /// Network-layer message name, if this is a network message.
    #[getter]
    fn network_message(&self) -> Option<&str> {
        self.summary.network_message.as_deref()
    }
    /// Application PDU type, such as `Confirmed-Request`.
    #[getter]
    fn pdu(&self) -> Option<&str> {
        self.summary.pdu.as_deref()
    }
    /// Application invoke identifier, when the PDU carries one.
    #[getter]
    fn invoke_id(&self) -> Option<u8> {
        self.summary.invoke_id
    }
    /// Service name in kebab case, such as `read-property`.
    #[getter]
    fn service(&self) -> Option<&str> {
        self.summary.service.as_deref()
    }
    /// Protocol refusal or malformed-header diagnostic; not a stable code.
    #[getter]
    fn problem(&self) -> Option<&str> {
        self.summary.problem.as_deref()
    }
    /// Header decode failure only. A valid Error, Reject, Abort or NAK
    /// has a `problem` but no `parse_error`. Service payloads are not fully validated.
    #[getter]
    fn parse_error(&self) -> Option<&str> {
        self.summary.parse_error.as_deref()
    }
    /// Device instance announced by an I-Am that could be decoded.
    #[getter]
    fn device(&self) -> Option<u32> {
        self.summary.device
    }
}

/// The links the decoder reads. Their tokens are `Frame.link`'s, from
/// [`link_token`], the one place the bindings spell them.
const DECODED: [(DataLink, Framing); 4] = [
    (DataLink::Bip, Framing::Bip),
    (DataLink::Bip6, Framing::Bip6),
    (DataLink::Sc, Framing::Sc),
    (DataLink::Mstp, Framing::Mstp),
];

/// The framing a `Frame.link` token names, exactly as `Frame.link` writes
/// it. `"other"`, a display name or any other text is refused, with the
/// tokens accepted.
fn framing(token: &str) -> PyResult<(Framing, &'static str)> {
    DECODED
        .iter()
        .find(|(link, _)| link_token(*link) == token)
        .map(|(link, framing)| (*framing, link_token(*link)))
        .ok_or_else(|| {
            let accepted: Vec<String> = DECODED
                .iter()
                .map(|(link, _)| format!("{:?}", link_token(*link)))
                .collect();
            PyValueError::new_err(format!(
                "link must be a Frame.link token the decoder reads, one of {}; got {token:?}",
                accepted.join(", ")
            ))
        })
}

/// Decode link, NPDU and APDU headers without sending traffic. Malformed
/// octets return a partial summary with `parse_error`, not a Python parser.
/// `link` has no default: octets read as the wrong link come back as a
/// parse failure, never as another link's message. It takes `Frame.link`'s
/// tokens (`"bip"`, `"ipv6"`, `"sc"`, `"mstp"`), so `link=frame.link` works.
#[pyfunction]
#[pyo3(signature = (octets, link))]
fn decode_frame(octets: &[u8], link: &str) -> PyResult<PyFrameSummary> {
    let (framing, link) = framing(link)?;
    Ok(PyFrameSummary {
        summary: summarize_frame(framing, octets),
        octets: octets.into(),
        link,
    })
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
            for refused in ["other", "bip6", "BIP", "BACnet/IP", "MS/TP", ""] {
                let error = decode.call1((REQUEST, refused)).unwrap_err();
                assert!(error.is_instance_of::<PyValueError>(py), "{refused}");
                assert!(
                    error
                        .to_string()
                        .contains(r#"one of "bip", "ipv6", "sc", "mstp""#),
                    "{error}"
                );
            }
            let positional = decode.call1((REQUEST, "bip")).unwrap();
            let kwargs = PyDict::new(py);
            kwargs.set_item("link", "bip").unwrap();
            let keyword = decode.call((REQUEST,), Some(&kwargs)).unwrap();
            assert!(positional.eq(&keyword).unwrap());
            let message: String = keyword.getattr("link_message").unwrap().extract().unwrap();
            assert_eq!(message, "original-unicast-npdu");
        });
    }

    /// Every token `Frame.link` writes for a link the decoder reads selects
    /// that link's framing; every other token is refused.
    #[test]
    fn frame_link_tokens_select_their_framing() {
        for (link, framing) in DECODED {
            let token = link_token(link);
            assert_eq!(super::framing(token).unwrap(), (framing, token));
        }
        Python::initialize();
        for link in [DataLink::Ethernet, DataLink::Loopback] {
            assert_eq!(link_token(link), "other");
            assert!(super::framing(link_token(link)).is_err());
        }
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
