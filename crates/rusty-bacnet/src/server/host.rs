//! Where a Python `BACnetServer`'s running server lives (#2617).
//!
//! A standalone server owns its Rust server: `start()` publishes it and
//! `stop()` takes it. An endpoint's server, from `await endpoint.server()`,
//! is the one the endpoint's session owns: each call borrows it through the
//! endpoint's session lock, a read guard held for the call as a standalone
//! server's own read guard is. The endpoint's `close()` takes that lock
//! whole, so it waits for the calls in flight, and the calls after it find
//! no server and raise. A call holds no Python: the bindings attach only
//! once it has returned. The handle names the run it came from, so one
//! kept past a close never reaches a later start's server, and holds the
//! session weakly, so it never keeps a dropped endpoint's session alive.

use std::sync::{Arc, Weak};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use tokio::sync::{OwnedRwLockReadGuard, RwLock, RwLockReadGuard};

use crate::app_transport::AppTransport;
use crate::endpoint::Running;

/// The Rust server a Python `BACnetServer` calls.
type Server = bacnet_server::server::BACnetServer<AppTransport>;

/// What a Python `BACnetServer`'s calls reach their server through.
#[derive(Clone)]
pub(crate) enum ServerHost {
    /// A standalone server's own, which its `start()` publishes.
    Own(Arc<RwLock<Option<Server>>>),
    /// An endpoint's, borrowed per call from its session (#2617).
    Endpoint(SeatedHost),
}

/// An endpoint's session, and the run of it whose server this is: a server
/// handed out before a close never reaches the server of a later start.
/// Weak, so a server kept past its endpoint keeps no session alive: the
/// endpoint dropped without a close drops its session as before.
#[derive(Clone)]
pub(crate) struct SeatedHost {
    pub(crate) session: Weak<RwLock<Option<Running>>>,
    pub(crate) run: u64,
}

/// A read guard of the server's host, held for one call. An endpoint's is
/// `None` once the endpoint is gone.
pub(crate) enum HostGuard<'a> {
    Own(RwLockReadGuard<'a, Option<Server>>),
    Endpoint(Option<OwnedRwLockReadGuard<Option<Running>>>, u64),
}

impl ServerHost {
    /// A standalone server's host, empty until its `start()`.
    pub(crate) fn own() -> Self {
        Self::Own(Arc::new(RwLock::new(None)))
    }

    /// Whether this is an endpoint's server.
    pub(crate) fn is_seated(&self) -> bool {
        matches!(self, Self::Endpoint(_))
    }

    /// A standalone server's own slot, for its `start()` and `stop()`; an
    /// endpoint's server refuses `what` with a RuntimeError.
    pub(crate) fn owned(&self, what: &str) -> PyResult<Arc<RwLock<Option<Server>>>> {
        match self {
            Self::Own(inner) => Ok(Arc::clone(inner)),
            Self::Endpoint(_) => Err(seated_refusal(what)),
        }
    }

    /// Share the host for one call. An endpoint's waits while the endpoint
    /// starts or closes, as a standalone server's waits for its `stop()`.
    pub(crate) async fn read(&self) -> HostGuard<'_> {
        match self {
            Self::Own(inner) => HostGuard::Own(inner.read().await),
            Self::Endpoint(seated) => {
                let guard = match seated.session.upgrade() {
                    Some(session) => Some(session.read_owned().await),
                    None => None,
                };
                HostGuard::Endpoint(guard, seated.run)
            }
        }
    }
}

impl HostGuard<'_> {
    /// The running server, or the RuntimeError its call raises without one.
    pub(crate) fn server(&self) -> PyResult<&Server> {
        match self {
            Self::Own(guard) => guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started")),
            Self::Endpoint(guard, run) => guard
                .as_deref()
                .and_then(Option::as_ref)
                .and_then(|running| running.server_of_run(*run))
                .ok_or_else(endpoint_closed),
        }
    }

    /// Record a reset the application makes for its own reasons (#1837):
    /// a standalone server through its target Audit Reporters, an
    /// endpoint's through the session's source Reporter, the one the
    /// endpoint's server reports its peers' resets through (#2617); with
    /// none, nothing. The server must run, as for any other call.
    pub(crate) async fn report_device_reset_local(&self) -> PyResult<()> {
        let server = self.server()?;
        match self {
            Self::Own(_) => server
                .report_device_reset_local()
                .await
                .map_err(crate::errors::to_py_err),
            Self::Endpoint(guard, _) => {
                let running = guard
                    .as_deref()
                    .and_then(Option::as_ref)
                    .ok_or_else(endpoint_closed)?;
                running
                    .report_device_reset_local()
                    .await
                    .map_err(crate::errors::to_py_err)
            }
        }
    }
}

/// An endpoint's server after the endpoint's close (or its drop).
fn endpoint_closed() -> PyErr {
    PyRuntimeError::new_err(
        "the endpoint this server came from is closed, and its server stopped with it; \
         after a restart, await endpoint.server() for the new one",
    )
}

impl super::BACnetServer {
    /// The `BACnetServer` `await endpoint.server()` returns (#2617): the
    /// server the endpoint's session owns, on a `transport` link (`"bip"`,
    /// `"sc"` or `"mstp"`), whose addresses `local_address()` formats as a
    /// standalone server on that link does. It is started already, so the
    /// methods that configure a server before its start refuse, as on a
    /// running standalone server; the endpoint's constructor and `add_*`
    /// calls configure it. The settings a standalone server's start reads
    /// keep their defaults here, unused.
    pub(crate) fn seated(host: SeatedHost, transport: &str, device_instance: u32) -> Self {
        let mut server = Self::unconfigured(ServerHost::Endpoint(host), device_instance);
        server.transport_type = transport.to_string();
        server
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        server
            .forwarding_configuration_started
            .store(true, std::sync::atomic::Ordering::Release);
        server
    }

    /// The host this server's calls go through, for the tests that hold a
    /// call open.
    #[cfg(test)]
    pub(crate) fn host_for_test(&self) -> &ServerHost {
        &self.inner
    }

    /// A server on `host` with every start setting at its default.
    pub(crate) fn unconfigured(host: ServerHost, device_instance: u32) -> Self {
        use bacnet_server::server;
        use std::sync::atomic::AtomicBool;
        Self {
            registered_network_port: None,
            inner: host,
            device_instance,
            segmentation_supported: bacnet_types::enums::Segmentation::NO_SEGMENTATION,
            apdu_segment_timeout_ms: 5000,
            device_name: String::new(),
            transport_type: "bip".into(),
            interface: std::net::Ipv4Addr::UNSPECIFIED,
            port: 0,
            broadcast_address: String::new(),
            share_port_by_address: false,
            share_wildcard_port: false,
            bip_mode: Default::default(),
            bip6_foreign: None,
            multicast: None,
            sc_hub: None,
            sc_vmac: None,
            sc_device_uuid: [0; 16],
            sc_npdu_admission_policy: Default::default(),
            sc_ca_cert: None,
            sc_client_cert: None,
            sc_client_key: None,
            sc_heartbeat_interval_ms: None,
            sc_heartbeat_timeout_ms: None,
            ipv6_interface: None,
            mstp: crate::mstp_py::MstpOptions {
                serial_port: None,
                baud: 38_400,
                mac: 1,
                max_master: 127,
                max_info_frames: 1,
                extended_frames: false,
                echo: false,
                echo_grace_ms: None,
                timing: crate::mstp_py::MstpTiming::default(),
            },
            dcc_password: None,
            mutation_policy: bacnet_server::mutation::MutationPolicy::default(),
            mutation_callback: None,
            dcc_policy: server::DccPolicy::default(),
            dcc_source_restriction: None,
            dcc_disable_rate_limit: None,
            reinit_password: None,
            reinitialize: None,
            request_admission_policy: server::RequestAdmissionPolicy::default(),
            read_property_multiple_budget:
                bacnet_device_core::read::ReadPropertyMultipleBudget::default(),
            get_alarm_summary_budget: server::GetAlarmSummaryBudget::default(),
            get_enrollment_summary_budget: server::GetEnrollmentSummaryBudget::default(),
            atomic_read_file_budget: server::AtomicReadFileBudget::default(),
            atomic_write_file_budget: server::AtomicWriteFileBudget::default(),
            read_range_budget: server::ReadRangeBudget::default(),
            get_event_information_budget:
                bacnet_device_core::event::GetEventInformationBudget::default(),
            cov_policy: bacnet_device_core::cov::CovPolicy::default(),
            time_sync_policy: server::TimeSyncPolicy::default(),
            clock_zone: server::ClockConfig::default(),
            audit_notification_sink: None,
            audit_reporters: None,
            audit_recipient: std::sync::Mutex::new(None),
            device_bindings: std::collections::BTreeMap::new(),
            forwarding_configuration_started: AtomicBool::new(false),
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
            pending_forwarder_save_counters: std::sync::Mutex::new(Vec::new()),
            forwarder_save_counters: Arc::new(std::sync::Mutex::new(Vec::new())),
            router: None,
        }
    }
}

/// What an endpoint's server refuses (`call`, its `start()` or `stop()`):
/// its endpoint starts and stops it.
fn seated_refusal(call: &str) -> PyErr {
    PyRuntimeError::new_err(format!(
        "{call} refused: this BACnetServer is the endpoint's, which starts it with \
         its own start and stops it with its close()"
    ))
}
