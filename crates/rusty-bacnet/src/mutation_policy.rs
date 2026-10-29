//! A Python mutation policy (#1966): `BACnetServer(mutation_policy=...)`
//! takes `"permissive"`, `"deny_all"` or a callable that receives a frozen,
//! redacted `MutationContext` and returns a bool. The endpoints'
//! `device_write_policy` keyword (#2033) takes a callable under the same
//! rules, deciding writes to the endpoint's local Device.
//!
//! The callable stands in for the Rust `ServerPolicy::authorize_mutation`,
//! so it is asked wherever that is: each covered confirmed service, each
//! WritePropertyMultiple element and each Channel an inbound WriteGroup would
//! write. It runs on the server's dispatch path, entered through
//! [`crate::py_async::attach`], with no database guard held: a
//! WritePropertyMultiple asks about its elements before it takes the write
//! lock (#2032), and on Tokio's blocking pool while the request waits. A
//! callable that blocks (a `run_coroutine_threadsafe(...).result()`, say)
//! delays only its own request, not other clients or `stop()`, which aborts
//! that request without waiting. It should still decide from the context
//! alone, synchronously and quickly. [`crate::py_async::under_native_lock`]
//! still refuses any binding operation it starts, since it runs inside the
//! server's dispatch.
//!
//! It fails closed: an exception (shown through `sys.unraisablehook`), a
//! return value that isn't a bool, and an exiting interpreter all deny. The
//! denied mutation changes nothing, but the elements of a
//! WritePropertyMultiple before a denied one stay written, and the callable
//! can be asked about an element the request never reaches, when an earlier
//! one fails as it is written.

use std::sync::Arc;

use bacnet_server::mutation::{
    DeviceWriteAuthorizer, MutationAuthorizationContext, MutationPolicy, MutationTarget,
};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyString};

use crate::types::{PyObjectIdentifier, PyObjectType, PyPropertyIdentifier};

/// The `mutation_policy` keyword: the native mode, and the callable that
/// decides each mutation when one was given (in permissive mode, so that it
/// is asked). The default is `"permissive"`.
#[derive(Clone, Default)]
pub(crate) struct MutationSetting {
    pub(crate) mode: MutationPolicy,
    pub(crate) callback: Option<Arc<PyMutationPolicy>>,
}

/// Parse the `mutation_policy` keyword: a mode name or a callable.
pub(crate) fn parse(value: &Bound<'_, PyAny>) -> PyResult<MutationSetting> {
    if let Ok(name) = value.cast::<PyString>() {
        let mode = match name.to_str()? {
            "permissive" => MutationPolicy::Permissive,
            "deny_all" => MutationPolicy::DenyAll,
            _ => {
                return Err(PyValueError::new_err(
                    "mutation_policy must be 'permissive', 'deny_all' or a callable",
                ));
            }
        };
        return Ok(MutationSetting {
            mode,
            callback: None,
        });
    }
    if value.is_callable() {
        return Ok(MutationSetting {
            mode: MutationPolicy::Permissive,
            callback: Some(Arc::new(PyMutationPolicy {
                callable: value.clone().unbind(),
                keyword: "mutation_policy",
            })),
        });
    }
    Err(PyTypeError::new_err(
        "mutation_policy must be 'permissive', 'deny_all' or a callable",
    ))
}

/// A server's Python mutation callable, or an endpoint's Device-write one.
pub(crate) struct PyMutationPolicy {
    callable: Py<PyAny>,
    /// The keyword it came from, which its errors name.
    keyword: &'static str,
}

impl PyMutationPolicy {
    /// Whether to carry out the mutation `context` describes. Every failure
    /// denies.
    pub(crate) fn decide(&self, context: &MutationAuthorizationContext) -> bool {
        let context = PyMutationContext::new(context);
        crate::py_async::attach(|py| {
            let callable = self.callable.bind(py);
            let returned =
                crate::py_async::under_native_lock(|| callable.call1((context,)));
            Ok(match returned.and_then(|value| verdict(&value, self.keyword)) {
                Ok(allow) => allow,
                Err(error) => {
                    // Shown through sys.unraisablehook, as asyncio shows a
                    // callback's exception.
                    error.write_unraisable(py, Some(callable));
                    false
                }
            })
        })
        // The interpreter is exiting: nobody can decide, so deny.
        .unwrap_or(false)
    }
}

/// The callable's answer: exactly `True` or `False`. Anything else, a
/// truthy object or a coroutine included, is a TypeError and denies.
fn verdict(value: &Bound<'_, PyAny>, keyword: &str) -> PyResult<bool> {
    if let Ok(answer) = value.cast::<PyBool>() {
        return Ok(answer.is_true());
    }
    let py = value.py();
    if py
        .import(intern!(py, "inspect"))?
        .call_method1(intern!(py, "isawaitable"), (value,))?
        .is_truthy()?
    {
        // Closed, so it never warns that it was never awaited.
        if value.hasattr(intern!(py, "close"))? {
            value.call_method0(intern!(py, "close"))?;
        }
        return Err(PyTypeError::new_err(format!(
            "the {keyword} callable must be a plain function returning a bool"
        )));
    }
    Err(PyTypeError::new_err(format!(
        "the {keyword} callable must return a bool"
    )))
}

/// An endpoint's `device_write_policy` keyword (#2033): a callable that
/// decides each WriteProperty to the endpoint's local Device (its
/// Description or installed Audit recipient), as the native session's
/// [`DeviceWriteAuthorizer`] does. It gets the same `MutationContext` and
/// follows the same fail-closed rules as a server's `mutation_policy`
/// callable. Without it, the endpoint executes no WriteProperty.
#[derive(Clone, Default)]
pub(crate) struct DeviceWritePolicy(Option<Arc<PyMutationPolicy>>);

impl DeviceWritePolicy {
    /// Parse the keyword: None, or a callable.
    pub(crate) fn parse(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let Some(value) = value.filter(|value| !value.is_none()) else {
            return Ok(Self::default());
        };
        if !value.is_callable() {
            return Err(PyTypeError::new_err(
                "device_write_policy must be a callable or None",
            ));
        }
        Ok(Self(Some(Arc::new(PyMutationPolicy {
            callable: value.clone().unbind(),
            keyword: "device_write_policy",
        }))))
    }

    /// Whether the server role executes WriteProperty to the local Device.
    pub(crate) fn enabled(&self) -> bool {
        self.0.is_some()
    }

    /// Install the callable as the session's Device-write authorizer.
    pub(crate) fn install<T>(
        &self,
        session: bacnet_endpoint::session::EndpointSession<T>,
    ) -> bacnet_endpoint::session::EndpointSession<T>
    where
        T: bacnet_transport::port::TransportPort + 'static,
    {
        let Some(policy) = self.0.clone() else {
            return session;
        };
        let authorizer: DeviceWriteAuthorizer = Arc::new(move |context| policy.decide(context));
        session.with_device_writes(authorizer)
    }
}

/// What a mutation decision is about, as `MutationContext` shows it: the
/// target's coordinates, never its values or file data.
#[derive(Clone, Default)]
struct Target {
    object_identifier: Option<ObjectIdentifier>,
    object_type: Option<ObjectType>,
    property_identifier: Option<PropertyIdentifier>,
    array_index: Option<u32>,
    priority: Option<u8>,
}

impl Target {
    fn object(object: ObjectIdentifier) -> Self {
        Self {
            object_identifier: Some(object),
            object_type: Some(object.object_type()),
            ..Self::default()
        }
    }

    fn property(
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Self {
        Self {
            property_identifier: Some(property),
            array_index: index,
            ..Self::object(object)
        }
    }

    fn new(target: &MutationTarget) -> Self {
        match target {
            MutationTarget::WriteProperty(request) => Self {
                priority: request.priority,
                ..Self::property(
                    request.object_identifier,
                    request.property_identifier,
                    request.property_array_index,
                )
            },
            MutationTarget::WritePropertyMultiple(attempt) => Self {
                priority: attempt.priority,
                ..Self::property(
                    attempt.reference.object_identifier,
                    PropertyIdentifier::from_raw(attempt.reference.property_identifier),
                    attempt.reference.property_array_index,
                )
            },
            MutationTarget::CreateObject(request) => match request.object_specifier {
                ObjectSpecifier::Identifier(object) => Self::object(object),
                ObjectSpecifier::Type(object_type) => Self {
                    object_type: Some(object_type),
                    ..Self::default()
                },
            },
            MutationTarget::DeleteObject(request) => Self::object(request.object_identifier),
            MutationTarget::AddListElement(request)
            | MutationTarget::RemoveListElement(request) => Self::property(
                request.object_identifier,
                request.property_identifier,
                request.property_array_index,
            ),
            MutationTarget::AtomicWriteFile(request) => Self::object(request.file_identifier),
            MutationTarget::SubscribeCov(request) => {
                Self::object(request.monitored_object_identifier)
            }
            MutationTarget::SubscribeCovProperty(request) => Self::property(
                request.monitored_object_identifier,
                request.monitored_property_identifier,
                request.monitored_property_array_index,
            ),
            // Several objects: the context names none.
            MutationTarget::SubscribeCovPropertyMultiple(_) => Self::default(),
            MutationTarget::WriteGroup(target) => Self {
                priority: Some(target.priority),
                ..Self::object(target.channel)
            },
        }
    }
}

/// How the request reached this node, without identity material.
fn provenance_kind(provenance: &TransportProvenance) -> &'static str {
    if provenance.is_direct_peer() {
        "direct_peer"
    } else if provenance.is_hub_channel() {
        "hub_channel"
    } else if provenance.is_relayed_origin() {
        "relayed_origin"
    } else {
        "unverified"
    }
}

/// One mutation a Python `mutation_policy` callable is asked to allow: a
/// frozen copy of the redacted Rust context. Its repr shows the service and
/// invoke ID only.
#[pyclass(
    name = "MutationContext",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyMutationContext {
    service: String,
    target: Target,
    source_mac: MacAddr,
    source_network: Option<(u16, MacAddr)>,
    provenance: &'static str,
    principal: Option<[u8; 32]>,
    forwarding_peer_principal: Option<[u8; 32]>,
    invoke_id: Option<u8>,
}

impl PyMutationContext {
    fn new(context: &MutationAuthorizationContext) -> Self {
        Self {
            service: context.target.kind().replace('-', "_"),
            target: Target::new(&context.target),
            source_mac: context.source_mac.clone(),
            source_network: context
                .source_network
                .as_ref()
                .map(|source| (source.network, source.mac_address.clone())),
            provenance: provenance_kind(&context.provenance),
            principal: context.principal().map(|principal| principal.leaf_sha256()),
            forwarding_peer_principal: context
                .forwarding_peer_principal()
                .map(|principal| principal.leaf_sha256()),
            invoke_id: context.invoke_id,
        }
    }
}

#[pymethods]
impl PyMutationContext {
    /// The service, in snake case: `"write_property"`,
    /// `"write_property_multiple"` (one element), `"create_object"`,
    /// `"delete_object"`, `"add_list_element"`, `"remove_list_element"`,
    /// `"atomic_write_file"`, `"subscribe_cov"`, `"subscribe_cov_property"`,
    /// `"subscribe_cov_property_multiple"` or `"write_group"` (one Channel).
    #[getter]
    fn service(&self) -> &str {
        &self.service
    }

    /// The target object: the Channel for WriteGroup, the File for
    /// AtomicWriteFile. None for a CreateObject that names only a type and
    /// for SubscribeCOVPropertyMultiple.
    #[getter]
    fn object_identifier(&self) -> Option<PyObjectIdentifier> {
        self.target
            .object_identifier
            .map(PyObjectIdentifier::from_rust)
    }

    /// The target object's type, also for a CreateObject that names only a
    /// type. None for SubscribeCOVPropertyMultiple.
    #[getter]
    fn object_type(&self) -> Option<PyObjectType> {
        self.target.object_type.map(|inner| PyObjectType { inner })
    }

    /// The property written, listed or subscribed to, else None.
    #[getter]
    fn property_identifier(&self) -> Option<PyPropertyIdentifier> {
        self.target
            .property_identifier
            .map(|inner| PyPropertyIdentifier { inner })
    }

    /// The property's array index, when the request gives one.
    #[getter]
    fn array_index(&self) -> Option<u32> {
        self.target.array_index
    }

    /// The write priority, for WriteProperty, a WritePropertyMultiple element
    /// or WriteGroup, when there is one.
    #[getter]
    fn priority(&self) -> Option<u8> {
        self.target.priority
    }

    /// The immediate link source: the requester, or the last router.
    #[getter]
    fn source_mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.source_mac)
    }

    /// The originating network a routed request claims, else None.
    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.source_network.as_ref().map(|(network, _)| *network)
    }

    /// The originating MAC a routed request claims, else None.
    #[getter]
    fn source_address<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.source_network
            .as_ref()
            .map(|(_, mac)| PyBytes::new(py, mac))
    }

    /// How the request arrived: `"unverified"` (B/IP, B/IPv6, MS/TP,
    /// Ethernet), `"relayed_origin"` (through a BACnet/SC hub),
    /// `"hub_channel"` or `"direct_peer"` (a verified direct SC connection).
    #[getter]
    fn provenance(&self) -> &'static str {
        self.provenance
    }

    /// The SHA-256 of the verified leaf certificate of the peer that
    /// originated the request, when it came over a direct BACnet/SC
    /// connection without a routed source; None otherwise. A Python server
    /// has no direct SC listener, so this is None today.
    #[getter]
    fn principal<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.principal
            .as_ref()
            .map(|principal| PyBytes::new(py, principal))
    }

    /// The SHA-256 of the verified leaf certificate of the direct SC peer
    /// that handed the request over, a router included; None otherwise.
    #[getter]
    fn forwarding_peer_principal<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.forwarding_peer_principal
            .as_ref()
            .map(|principal| PyBytes::new(py, principal))
    }

    /// The originating peer's verified leaf certificate. Always None: a
    /// Python server has no direct BACnet/SC listener, and requests that a
    /// hub relays, routed requests and other transports carry no
    /// certificate.
    #[getter]
    fn certificate(&self) -> Option<Py<PyAny>> {
        None
    }

    /// The forwarding direct SC peer's verified leaf certificate. Always
    /// None, for the reason `certificate` gives.
    #[getter]
    fn forwarding_peer_certificate(&self) -> Option<Py<PyAny>> {
        None
    }

    /// A confirmed request's invoke ID; None for WriteGroup.
    #[getter]
    fn invoke_id(&self) -> Option<u8> {
        self.invoke_id
    }

    fn __repr__(&self) -> String {
        match self.invoke_id {
            Some(id) => format!(
                "MutationContext(service='{}', invoke_id={id})",
                self.service
            ),
            None => format!("MutationContext(service='{}')", self.service),
        }
    }

    /// Refuses `copy` and `pickle`, like the other values the bindings hand
    /// to callbacks. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Register `MutationContext` with the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMutationContext>()
}
