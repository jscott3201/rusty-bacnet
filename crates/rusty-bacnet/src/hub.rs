//! Python bindings for the BACnet/SC Hub.
//!
//! The hub is the central relay in a BACnet/SC (Secure Connect) topology.
//! It accepts TLS WebSocket connections from SC nodes and relays messages
//! between them per ASHRAE 135-2020 Annex AB.

use std::sync::Arc;
use std::time::Duration;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tokio::sync::Mutex;

use bacnet_transport::sc_hub::{
    ScHub, ScHubAdmissionDecision, ScHubAdmissionLimits, ScHubGracefulTimeouts,
    ScHubHandshakeTimeouts, ScHubRegistrationKind, ScHubShutdownOutcome,
};

use crate::errors::to_py_err;

/// BACnet/SC Hub — relays messages between SC nodes over TLS WebSocket.
///
/// Usage:
/// ```python
/// hub = ScHub(
///     listen="127.0.0.1:0",
///     cert="server.pem",
///     key="server.key",
///     ca_cert="ca.pem",       # required trusted issuer CA for mTLS
///     vmac=b"\x00\x00\x00\x00\x00\x01",
///     device_uuid=provisioned_hub_uuid,  # caller's durable lifetime identity
/// )
/// await hub.start()
/// print(f"Hub listening on {await hub.url()}")
/// # ... hub relays SC traffic ...
/// print(await hub.status())
/// print(await hub.shutdown_gracefully())  # "graceful" or "forced"
/// ```
///
/// The hub also works as an async context manager, which starts on entry and
/// forcefully stops on exit:
///
/// ```python
/// async with ScHub(..., device_uuid=provisioned_hub_uuid) as hub:
///     ...
/// # hub is forcefully stopped here, even if the body raised
/// ```
///
/// Lifecycle: `stop()` is forceful and idempotent (repeated calls are safe).
/// `shutdown_gracefully()` seals admission first, then drives the
/// Disconnect-Request/Ack plus WebSocket close exchange within the configured
/// graceful bounds, returning `"graceful"` only when every peer completed the
/// exchange and `"forced"` otherwise. Both consume the running hub: afterwards
/// the hub reads as not started (`status()` and a second graceful shutdown
/// raise `RuntimeError`; `stop()` stays a safe no-op). Dropping the object
/// without awaiting close only requests forceful-seal cleanup on a running
/// runtime and cannot guarantee awaited close — always await `stop()`,
/// `shutdown_gracefully()`, or context-manager exit when cleanup matters.
///
/// Admission policy is a static string only (`"allow_all"` default,
/// `"deny_all"`, or `"deny_uuid_replacement"`); arbitrary Python callables are rejected.
/// Refusing UUID replacement is a local security policy before protocol acceptance,
/// not the default Annex AB known-UUID replacement behavior or identity proof. The native policy
/// runs synchronously under the Tokio registry mutex, where attaching the
/// GIL could deadlock, so no Python callback can be installed there.
#[pyclass(name = "ScHub")]
pub struct PyScHub {
    inner: Arc<Mutex<Option<ScHub>>>,
    config: HubConfig,
    address: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Copy)]
enum AdmissionPolicy {
    AllowAll,
    DenyAll,
    DenyUuidReplacement,
}

impl AdmissionPolicy {
    fn evaluate(self, kind: ScHubRegistrationKind) -> ScHubAdmissionDecision {
        match (self, kind) {
            (Self::DenyAll, _)
            | (
                Self::DenyUuidReplacement,
                ScHubRegistrationKind::SameUuidSameVmac | ScHubRegistrationKind::SameUuidMovedVmac,
            ) => ScHubAdmissionDecision::Deny,
            _ => ScHubAdmissionDecision::Allow,
        }
    }
}

/// Owned, validated hub startup configuration shared by `start` and `__aenter__`.
///
/// Everything here is validated in the constructor, before any file I/O or
/// bind: the async start path only reuses these values through the native
/// `with_*` chain into `start_with_tls_config`.
#[derive(Clone)]
struct HubConfig {
    listen: String,
    cert: String,
    key: String,
    ca_cert: String,
    vmac: [u8; 6],
    device_uuid: [u8; 16],
    admission_limits: ScHubAdmissionLimits,
    graceful_timeouts: ScHubGracefulTimeouts,
    handshake_timeouts: ScHubHandshakeTimeouts,
    admission_policy: AdmissionPolicy,
}

impl HubConfig {
    /// Build the constrained native TLS policy, apply the validated bounds
    /// and static policy, and bind. Sync file reads happen here (at start),
    /// never in the constructor.
    async fn start(&self) -> PyResult<(ScHub, String)> {
        let mut server_tls =
            crate::tls::build_server_tls_config(&self.cert, &self.key, &self.ca_cert)
                .map_err(to_py_err)?
                .with_admission_limits(self.admission_limits)
                .with_graceful_timeouts(self.graceful_timeouts);
        // One native policy consumes the locked classification; no registry
        // copies, Python callbacks or certificate-principal inference.
        let policy = self.admission_policy;
        server_tls =
            server_tls.with_admission_policy(move |input| policy.evaluate(input.registration));

        let hub = ScHub::start_with_tls_config(
            &self.listen,
            server_tls,
            self.vmac,
            self.device_uuid,
            self.handshake_timeouts,
        )
        .await
        .map_err(to_py_err)?;

        let addr = hub
            .local_addr()
            .ok_or_else(|| PyRuntimeError::new_err("hub has no local address"))?
            .to_string();
        Ok((hub, addr))
    }
}

#[pymethods]
impl PyScHub {
    /// Create a new SC Hub.
    ///
    /// No I/O happens here: files are read and the socket is bound by
    /// `start()` (or `__aenter__`). Every argument below is validated now,
    /// before bind, in this order: `ca_cert` presence, VMAC length (the
    /// existing `RuntimeError`), reserved VMACs, `device_uuid`, admission
    /// limits, admission policy, graceful timeouts, then handshake timeouts.
    ///
    /// Args:
    ///     listen: Bind address, e.g. ``"127.0.0.1:0"`` for a random port.
    ///     cert: Path to server certificate PEM file.
    ///     key: Path to server private key PEM file.
    ///     ca_cert: Required nonempty path to trusted issuer CA PEM certificates.
    ///         Omitted, None, or empty values raise ValueError. The default only
    ///         preserves positional argument compatibility; it does not enable
    ///         server-auth-only TLS. Files are validated by start() before bind.
    ///     vmac: Hosting port's 6-byte VMAC, neither all zero nor all ff.
    ///     device_uuid: Required keyword-only nonzero 16-byte hosting device UUID.
    ///         Copied into owned storage. The caller must provision it before
    ///         deployment and durably reuse it for the device's lifetime. No UUID
    ///         generation, persistence, version/variant or certificate binding.
    ///         Identity errors precede file I/O; ca_cert presence is checked first.
    ///     max_clients: Keyword-only cap on simultaneously registered clients.
    ///         Zero or overflowing bounds raise ValueError (mirrors
    ///         `ScHubAdmissionLimits::new`); negative values raise OverflowError.
    ///     max_handshakes: Keyword-only cap on simultaneously unregistered
    ///         (handshake) connections. Same error mapping as `max_clients`.
    ///     admission_policy: Keyword-only static policy, `"allow_all"`
    ///         (default), `"deny_all"`, or `"deny_uuid_replacement"`. The latter
    ///         refuses same-UUID same/moved-VMAC replacement before protocol
    ///         acceptance; different-UUID collisions retain the standard NAK.
    ///         Unknown strings raise ValueError;
    ///         non-strings (including Python callables) raise TypeError — no
    ///         Python callback can run under the native registry lock.
    ///     graceful_disconnect_ack_ms: Keyword-only per-peer budget for sending
    ///         Disconnect-Request and awaiting Disconnect-Ack (default 5000).
    ///     graceful_ws_close_ms: Keyword-only per-peer AB.7.5.5 close-handshake
    ///         budget (default 5000).
    ///     graceful_overall_ms: Keyword-only whole-drain bound, must cover ack
    ///         + close (default 15000). Out-of-range values raise ValueError
    ///         (mirrors `ScHubGracefulTimeouts::new`).
    ///     handshake_tls_ms: Keyword-only budget from TCP admission through the
    ///         TLS handshake (default 10000).
    ///     handshake_websocket_upgrade_ms: Keyword-only budget from TLS success
    ///         through WebSocket acceptance (default 10000).
    ///     handshake_connect_request_ms: Keyword-only budget from WebSocket
    ///         acceptance through Connect-Request registration (default 10000).
    ///         Out-of-range values raise ValueError (mirrors
    ///         `ScHubHandshakeTimeouts::new`).
    #[new]
    #[pyo3(signature = (listen, cert, key, vmac, ca_cert=None, *, device_uuid=None, max_clients=256, max_handshakes=256, admission_policy="allow_all", graceful_disconnect_ack_ms=5000, graceful_ws_close_ms=5000, graceful_overall_ms=15000, handshake_tls_ms=10000, handshake_websocket_upgrade_ms=10000, handshake_connect_request_ms=10000))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        listen: &str,
        cert: &str,
        key: &str,
        vmac: Vec<u8>,
        ca_cert: Option<String>,
        device_uuid: Option<Vec<u8>>,
        max_clients: usize,
        max_handshakes: usize,
        admission_policy: &str,
        graceful_disconnect_ack_ms: u64,
        graceful_ws_close_ms: u64,
        graceful_overall_ms: u64,
        handshake_tls_ms: u64,
        handshake_websocket_upgrade_ms: u64,
        handshake_connect_request_ms: u64,
    ) -> PyResult<Self> {
        let ca_cert = ca_cert.filter(|path| !path.is_empty()).ok_or_else(|| {
            PyValueError::new_err("ca_cert must be a nonempty CA certificate path for mutual TLS")
        })?;
        if vmac.len() != 6 {
            return Err(PyRuntimeError::new_err("vmac must be exactly 6 bytes"));
        }
        let mut vmac_arr = [0u8; 6];
        vmac_arr.copy_from_slice(&vmac);
        if vmac_arr == [0; 6] || vmac_arr == [0xff; 6] {
            return Err(PyValueError::new_err(
                "vmac must not be UNKNOWN (all zero) or BROADCAST (all ff)",
            ));
        }
        let device_uuid: [u8; 16] = device_uuid
            .ok_or_else(|| PyValueError::new_err("device_uuid is required for ScHub"))?
            .try_into()
            .map_err(|_| PyValueError::new_err("device_uuid must be exactly 16 bytes"))?;
        if device_uuid == [0; 16] {
            return Err(PyValueError::new_err("device_uuid must not be all zero"));
        }
        let admission_limits = ScHubAdmissionLimits::new(max_clients, max_handshakes)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let admission_policy = match admission_policy {
            "allow_all" => AdmissionPolicy::AllowAll,
            "deny_all" => AdmissionPolicy::DenyAll,
            "deny_uuid_replacement" => AdmissionPolicy::DenyUuidReplacement,
            _ => {
                return Err(PyValueError::new_err(
                    "admission_policy must be 'allow_all', 'deny_all' or 'deny_uuid_replacement'",
                ));
            }
        };
        let graceful_timeouts = ScHubGracefulTimeouts::new(
            Duration::from_millis(graceful_disconnect_ack_ms),
            Duration::from_millis(graceful_ws_close_ms),
            Duration::from_millis(graceful_overall_ms),
        )
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let handshake_timeouts = ScHubHandshakeTimeouts::new(
            Duration::from_millis(handshake_tls_ms),
            Duration::from_millis(handshake_websocket_upgrade_ms),
            Duration::from_millis(handshake_connect_request_ms),
        )
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(None)),
            config: HubConfig {
                listen: listen.to_string(),
                cert: cert.to_string(),
                key: key.to_string(),
                ca_cert,
                vmac: vmac_arr,
                device_uuid,
                admission_limits,
                graceful_timeouts,
                handshake_timeouts,
                admission_policy,
            },
            address: Arc::new(Mutex::new(None)),
        })
    }

    /// Start the hub. Returns once the hub is listening.
    ///
    /// Credential files are loaded and the socket is bound here, applying the
    /// constructor-validated limits, static policy, and timeouts. Local TLS
    /// configuration failures raise `BacnetError`; repair the files and retry
    /// on the same hub.
    fn start<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let config = self.config.clone();
        let address = self.address.clone();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (hub, addr) = config.start().await?;

            *address.lock().await = Some(addr);
            *inner.lock().await = Some(hub);

            Ok(())
        })
    }

    /// Stop the hub.
    ///
    /// Forceful local shutdown: seals admission and aborts workers without
    /// the Disconnect exchange. Idempotent — stopping a stopped or never
    /// started hub is a safe no-op.
    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if let Some(mut hub) = inner.lock().await.take() {
                hub.stop().await;
            }
            Ok(())
        })
    }

    /// Shut the hub down gracefully, returning `"graceful"` or `"forced"`.
    ///
    /// Seals admission first, then drives the hub-initiated Disconnect-Request,
    /// awaited Disconnect-Ack, and AB.7.5.5 WebSocket close handshake per
    /// established peer within the configured graceful bounds, with a forceful
    /// fallback on expiry. Returns `"graceful"` only when every peer Ack was
    /// observed and the close handshake completed without timeout or abort;
    /// otherwise `"forced"`. Consumes the running hub like `stop()`: a second
    /// graceful shutdown raises `RuntimeError` (use `stop()` for an idempotent
    /// close). Cancelling this future leaves shutdown running; `stop()` stays
    /// safe to call afterwards.
    fn shutdown_gracefully<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut hub = {
                let mut guard = inner.lock().await;
                guard
                    .take()
                    .ok_or_else(|| PyRuntimeError::new_err("hub not started"))?
            };
            let outcome = hub.shutdown_gracefully().await;
            Ok(match outcome {
                ScHubShutdownOutcome::Graceful => "graceful",
                ScHubShutdownOutcome::Forced => "forced",
            })
        })
    }

    /// Bounded hub snapshot: listener state, handshake/client counts, and
    /// deny/drop counters. Counts and kind labels only — no certificates,
    /// keys, VMAC maps, or payloads. Raises `RuntimeError` before start and
    /// after stop, like the server counter accessors.
    fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let snapshot = {
                let guard = inner.lock().await;
                let hub = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("hub not started"))?;
                hub.status().await
            };
            // Owned snapshot only; the hub lock is released before touching
            // Python, and no await happens while the GIL is held.
            Python::attach(|py| {
                let dict = PyDict::new(py);
                dict.set_item("listening", snapshot.listening)?;
                dict.set_item("max_clients", snapshot.limits.max_clients)?;
                dict.set_item("max_handshakes", snapshot.limits.max_handshakes)?;
                dict.set_item("client_count", snapshot.client_count)?;
                dict.set_item("handshake_count", snapshot.handshake_count)?;
                dict.set_item("admin_denied", snapshot.admin_denied)?;
                dict.set_item(
                    "broadcast_sender_exhausted",
                    snapshot.broadcast_drops.sender_exhausted,
                )?;
                dict.set_item(
                    "broadcast_global_exhausted",
                    snapshot.broadcast_drops.global_exhausted,
                )?;
                Ok(dict.into_any().unbind())
            })
        })
    }

    /// Start the hub on context-manager entry, returning the hub itself.
    ///
    /// Shares the `start()` path (same validated config, same `BacnetError`
    /// mapping). Entering an already-started hub returns it without rebinding.
    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let self_ref = slf.clone().unbind();
        let inner = slf.borrow().inner.clone();
        let config = slf.borrow().config.clone();
        let address = slf.borrow().address.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            {
                let guard = inner.lock().await;
                if guard.is_some() {
                    return Ok(self_ref);
                }
            }
            let (hub, addr) = config.start().await?;

            *address.lock().await = Some(addr);
            *inner.lock().await = Some(hub);

            Ok(self_ref)
        })
    }

    /// Forcefully stop the hub on context-manager exit. Idempotent: exiting
    /// twice, or after an explicit `stop()`, is a safe no-op and never
    /// suppresses the body exception.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Bound<'py, PyAny>>,
        _exc_val: Option<Bound<'py, PyAny>>,
        _exc_tb: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            if let Some(mut hub) = inner.lock().await.take() {
                hub.stop().await;
            }
            Ok(())
        })
    }

    /// The address the hub is listening on (e.g. ``"127.0.0.1:47900"``).
    ///
    /// Returns ``None`` before ``start()`` is called.
    fn address<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let address = self.address.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = address.lock().await;
            Ok(guard.clone())
        })
    }

    /// The ``wss://`` URL for SC clients to connect to.
    ///
    /// Returns ``None`` before ``start()`` is called.
    fn url<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let address = self.address.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let guard = address.lock().await;
            let result: Option<String> = guard
                .as_ref()
                .map(|a| format!("wss://localhost:{}", a.rsplit(':').next().unwrap_or("0")));
            Ok(result)
        })
    }
}
