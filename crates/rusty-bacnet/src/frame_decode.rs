//! Python access to the shared, offline Rust frame-summary decoder.

use bacnet_ops::decode::frame::{FrameSummary, Framing, summarize_frame};
use bacnet_types::data_link::DataLink;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::frames::link_token;

/// A frozen header summary; unavailable fields remain `None`. Two
/// summaries are equal, and hash alike, when every field is. It holds the
/// fields only, never the frame's octets, so a password the frame carried
/// is not in it or in its pickle.
#[pyclass(name = "FrameSummary", module = "rusty_bacnet", frozen, eq, hash)]
#[derive(PartialEq, Eq, Hash)]
pub(crate) struct PyFrameSummary {
    summary: FrameSummary,
}

#[pymethods]
impl PyFrameSummary {
    /// The link message, then the PDU, service and invoke ID and the parse
    /// failure or problem, where the summary has them.
    fn __repr__(&self) -> String {
        let summary = &self.summary;
        let mut parts = vec![format!("link_message='{}'", summary.link_message)];
        if let Some(pdu) = &summary.pdu {
            parts.push(format!("pdu='{pdu}'"));
        }
        if let Some(service) = &summary.service {
            parts.push(format!("service='{service}'"));
        }
        if let Some(id) = summary.invoke_id {
            parts.push(format!("invoke_id={id}"));
        }
        if let Some(error) = &summary.parse_error {
            parts.push(format!("parse_error='{error}'"));
        } else if let Some(problem) = &summary.problem {
            parts.push(format!("problem='{problem}'"));
        }
        format!("FrameSummary({})", parts.join(", "))
    }

    /// Copy and pickle by value: `_rebuild` with the sixteen fields, in
    /// the order the stub lists them, and nothing of the frame.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let s = &slf.get().summary;
        let fields = [
            s.link_message.clone().into_pyobject(py)?.into_any(),
            s.broadcast.into_pyobject(py)?.to_owned().into_any(),
            s.link_source.clone().into_pyobject(py)?,
            s.link_destination.clone().into_pyobject(py)?,
            s.original_source.clone().into_pyobject(py)?,
            s.snet.into_pyobject(py)?,
            s.dnet.into_pyobject(py)?,
            s.routed_source.clone().into_pyobject(py)?,
            s.routed_destination.clone().into_pyobject(py)?,
            s.network_message.clone().into_pyobject(py)?,
            s.pdu.clone().into_pyobject(py)?,
            s.invoke_id.into_pyobject(py)?,
            s.service.clone().into_pyobject(py)?,
            s.problem.clone().into_pyobject(py)?,
            s.parse_error.clone().into_pyobject(py)?,
            s.device.into_pyobject(py)?,
        ];
        Ok((
            slf.get_type().getattr("_rebuild")?,
            PyTuple::new(py, fields)?,
        ))
    }

    /// A summary from a pickle's fields, as [`Self::__reduce__`] lists
    /// them. Nothing is decoded.
    #[staticmethod]
    fn _rebuild(
        link_message: String,
        broadcast: bool,
        link_source: Option<String>,
        link_destination: Option<String>,
        original_source: Option<String>,
        snet: Option<u16>,
        dnet: Option<u16>,
        routed_source: Option<String>,
        routed_destination: Option<String>,
        network_message: Option<String>,
        pdu: Option<String>,
        invoke_id: Option<u8>,
        service: Option<String>,
        problem: Option<String>,
        parse_error: Option<String>,
        device: Option<u32>,
    ) -> Self {
        Self {
            summary: FrameSummary {
                link_message,
                broadcast,
                link_source,
                link_destination,
                original_source,
                snet,
                dnet,
                network_message,
                pdu,
                invoke_id,
                service,
                problem,
                parse_error,
                device,
                routed_source,
                routed_destination,
            },
        }
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
pub(crate) fn framing(token: &str) -> PyResult<Framing> {
    DECODED
        .iter()
        .find(|(link, _)| link_token(*link) == token)
        .map(|(_, framing)| *framing)
        .ok_or_else(|| {
            let accepted: Vec<String> = DECODED
                .iter()
                .map(|(link, _)| format!("'{}'", link_token(*link)))
                .collect();
            PyValueError::new_err(format!(
                "link must be a Frame.link token the decoder reads, one of {}; got '{token}'",
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
    Ok(PyFrameSummary {
        summary: summarize_frame(framing(link)?, octets),
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
                        .contains("one of 'bip', 'ipv6', 'sc', 'mstp'"),
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
            assert_eq!(super::framing(link_token(link)).unwrap(), framing);
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
                "FrameSummary(link_message='original-unicast-npdu', pdu='Confirmed-Request', \
                 service='read-property', invoke_id=1)"
            );
            assert!(
                other
                    .repr()
                    .unwrap()
                    .to_string()
                    .starts_with("FrameSummary(link_message='malformed', parse_error='")
            );
        });
    }

    /// A DeviceCommunicationControl request on B/IP whose password is
    /// `secret123`.
    const DCC: &[u8] = &[
        0x81, 0x0A, 0x00, 0x18, 0x01, 0x04, 0x00, 0x05, 0x01, 0x11, 0x19, 0x00, 0x2D, 0x0A, 0x00,
        b's', b'e', b'c', b'r', b'e', b't', b'1', b'2', b'3',
    ];

    /// Copies are equal and hash alike, and what `copy` and `pickle` get
    /// is the summary's fields: none of the frame, so not its password.
    #[test]
    fn a_copy_carries_the_fields_and_not_the_frame() {
        Python::initialize();
        Python::attach(|py| {
            let decode = module(py).getattr("decode_frame").unwrap();
            let summary = decode.call1((DCC, "bip")).unwrap();
            let service: String = summary.getattr("service").unwrap().extract().unwrap();
            assert_eq!(service, "device-communication-control");
            let reduced = summary.call_method0("__reduce__").unwrap();
            let text = reduced.repr().unwrap().to_string();
            assert!(!text.contains("secret"), "{text}");
            assert_eq!(reduced.get_item(1).unwrap().len().unwrap(), 16);
            let copy = py.import("copy").unwrap();
            for how in ["copy", "deepcopy"] {
                let copied = copy.call_method1(how, (&summary,)).unwrap();
                assert!(copied.eq(&summary).unwrap(), "{how}");
                assert_eq!(copied.hash().unwrap(), summary.hash().unwrap(), "{how}");
                assert_eq!(
                    copied.repr().unwrap().to_string(),
                    summary.repr().unwrap().to_string()
                );
            }
        });
    }
}
