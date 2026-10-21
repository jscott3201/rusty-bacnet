//! Frozen Python copies of SC hub-connector events and link status, and the
//! async iterator over the events (#1829).

use std::time::Duration;

use bacnet_transport::sc::{
    ScActiveHub, ScConnectionState, ScDialStage, ScDisconnectCause, ScEvent, ScLinkStatus,
};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::event_stream::LossyReceiver;

fn hub_name(hub: ScActiveHub) -> &'static str {
    match hub {
        ScActiveHub::Primary => "primary",
        ScActiveHub::Failover => "failover",
    }
}

fn state_name(state: ScConnectionState) -> &'static str {
    match state {
        ScConnectionState::Disconnected => "disconnected",
        ScConnectionState::Connecting => "connecting",
        ScConnectionState::Connected => "connected",
        ScConnectionState::Disconnecting => "disconnecting",
    }
}

/// One hub-connector event, as Python reads it.
#[pyclass(name = "ScEvent", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScEvent(ScEvent);

impl PyScEvent {
    fn cause_of(&self) -> Option<&ScDisconnectCause> {
        match &self.0 {
            ScEvent::Disconnected { cause, .. }
            | ScEvent::ConnectFailed { cause, .. }
            | ScEvent::PrimaryRestoreFailed { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

#[pymethods]
impl PyScEvent {
    /// `"connected"`, `"disconnected"`, `"connect_failed"`,
    /// `"reconnect_attempt"`, `"failover_engaged"`, `"primary_restored"`,
    /// `"primary_restore_failed"` or `"reconnect_gave_up"`; `"other"` for
    /// an event this binding doesn't name yet.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.0 {
            ScEvent::Connected { .. } => "connected",
            ScEvent::Disconnected { .. } => "disconnected",
            ScEvent::ConnectFailed { .. } => "connect_failed",
            ScEvent::ReconnectAttempt { .. } => "reconnect_attempt",
            ScEvent::FailoverEngaged => "failover_engaged",
            ScEvent::PrimaryRestored => "primary_restored",
            ScEvent::PrimaryRestoreFailed { .. } => "primary_restore_failed",
            ScEvent::ReconnectGaveUp { .. } => "reconnect_gave_up",
            _ => "other",
        }
    }

    /// `"primary"` or `"failover"`: the hub connected to, lost, failed or
    /// being dialled; `"primary"` for a `"primary_restore_failed"` probe.
    /// `None` for the events that name no hub.
    #[getter]
    fn hub(&self) -> Option<&'static str> {
        match &self.0 {
            ScEvent::Connected { hub, .. }
            | ScEvent::Disconnected { hub, .. }
            | ScEvent::ConnectFailed { hub, .. }
            | ScEvent::ReconnectAttempt { hub, .. } => Some(hub_name(*hub)),
            ScEvent::PrimaryRestoreFailed { .. } => Some(hub_name(ScActiveHub::Primary)),
            _ => None,
        }
    }

    /// The hub's VMAC from its Connect-Accept, on `"connected"` only.
    #[getter]
    fn hub_vmac<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        match &self.0 {
            ScEvent::Connected { hub_vmac, .. } => Some(PyBytes::new(py, hub_vmac)),
            _ => None,
        }
    }

    /// Why a `"disconnected"` connection ended, or a `"connect_failed"`
    /// attempt or `"primary_restore_failed"` probe failed: `"local_close"`,
    /// `"peer_close"`, `"heartbeat_timeout"`, `"websocket"`, `"tls"`,
    /// `"nak"`, `"dial_timeout"`, `"connect_accept_timeout"`,
    /// `"upgrade_refused"`, `"malformed_result"`,
    /// `"unexpected_connect_reply"`, `"rejection_expired"` or `"other"`.
    #[getter]
    fn cause(&self) -> Option<&'static str> {
        self.cause_of().map(|cause| match cause {
            ScDisconnectCause::LocalClose => "local_close",
            ScDisconnectCause::PeerClose => "peer_close",
            ScDisconnectCause::HeartbeatTimeout => "heartbeat_timeout",
            ScDisconnectCause::WebSocket(_) => "websocket",
            ScDisconnectCause::Tls(_) => "tls",
            ScDisconnectCause::Nak { .. } => "nak",
            ScDisconnectCause::DialTimeout { .. } => "dial_timeout",
            ScDisconnectCause::ConnectAcceptTimeout => "connect_accept_timeout",
            ScDisconnectCause::UpgradeRefused { .. } => "upgrade_refused",
            ScDisconnectCause::MalformedResult => "malformed_result",
            ScDisconnectCause::UnexpectedConnectReply => "unexpected_connect_reply",
            ScDisconnectCause::RejectionExpired => "rejection_expired",
            _ => "other",
        })
    }

    /// Diagnostic text for a `"websocket"`, `"tls"` or `"other"` cause. Not
    /// stable: show it, don't match on it.
    #[getter]
    fn cause_detail(&self) -> Option<String> {
        match self.cause_of()? {
            ScDisconnectCause::WebSocket(kind) => Some(kind.to_string()),
            ScDisconnectCause::Tls(failure) => Some(format!("{failure:?}")),
            ScDisconnectCause::Other(text) => Some(text.clone()),
            _ => None,
        }
    }

    /// For a `"dial_timeout"` cause, the step the dial was in:
    /// `"host_lookup"`, `"tcp_connect"`, `"tls"` or `"websocket_upgrade"`;
    /// `None` when the dial did not say.
    #[getter]
    fn dial_stage(&self) -> Option<&'static str> {
        match self.cause_of()? {
            ScDisconnectCause::DialTimeout {
                stage: Some(stage), ..
            } => Some(match stage {
                ScDialStage::HostLookup => "host_lookup",
                ScDialStage::TcpConnect => "tcp_connect",
                ScDialStage::Tls => "tls",
                ScDialStage::WebSocketUpgrade => "websocket_upgrade",
                _ => "other",
            }),
            _ => None,
        }
    }

    /// For an `"upgrade_refused"` cause, the HTTP status the server
    /// answered the WebSocket upgrade with, such as a proxy's 502.
    #[getter]
    fn http_status(&self) -> Option<u16> {
        match self.cause_of()? {
            ScDisconnectCause::UpgradeRefused { http_status, .. } => Some(*http_status),
            _ => None,
        }
    }

    /// The NAK's error class, for a `"nak"` cause.
    #[getter]
    fn error_class(&self) -> Option<u16> {
        match self.cause_of()? {
            ScDisconnectCause::Nak { error_class, .. } => Some(error_class.to_raw()),
            _ => None,
        }
    }

    /// The NAK's error code, for a `"nak"` cause.
    #[getter]
    fn error_code(&self) -> Option<u16> {
        match self.cause_of()? {
            ScDisconnectCause::Nak { error_code, .. } => Some(error_code.to_raw()),
            _ => None,
        }
    }

    /// The attempt number, from 1, of a `"reconnect_attempt"`; for
    /// `"reconnect_gave_up"`, the attempts made.
    #[getter]
    fn attempt(&self) -> Option<u32> {
        match &self.0 {
            ScEvent::ReconnectAttempt { attempt, .. } => Some(*attempt),
            ScEvent::ReconnectGaveUp { attempts, .. } => Some(*attempts),
            _ => None,
        }
    }

    /// The jittered delay, in seconds, a `"reconnect_attempt"` waited.
    #[getter]
    fn backoff(&self) -> Option<f64> {
        match &self.0 {
            ScEvent::ReconnectAttempt { backoff, .. } => Some(backoff.as_secs_f64()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        let mut text = format!("ScEvent(kind='{}'", self.kind());
        if let Some(hub) = self.hub() {
            text.push_str(&format!(", hub='{hub}'"));
        }
        if let Some(cause) = self.cause() {
            text.push_str(&format!(", cause='{cause}'"));
        }
        if let Some(attempt) = self.attempt() {
            text.push_str(&format!(", attempt={attempt}"));
        }
        text.push(')');
        text
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A snapshot of the hub link.
#[pyclass(name = "ScLinkStatus", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScLinkStatus(pub(crate) ScLinkStatus);

fn seconds(duration: Option<Duration>) -> Option<f64> {
    duration.map(|duration| duration.as_secs_f64())
}

#[pymethods]
impl PyScLinkStatus {
    /// `"disconnected"`, `"connecting"`, `"connected"` or `"disconnecting"`.
    #[getter]
    fn state(&self) -> &'static str {
        state_name(self.0.state)
    }

    /// The hub in use or, while reconnecting, the one being dialled:
    /// `"primary"` or `"failover"`; `None` before the first attempt.
    #[getter]
    fn active_hub(&self) -> Option<&'static str> {
        self.0.active_hub.map(hub_name)
    }

    /// The connected hub's VMAC, `None` while not connected.
    #[getter]
    fn hub_vmac<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.0.hub_vmac.map(|vmac| PyBytes::new(py, &vmac))
    }

    /// Seconds since the last message accepted from the hub, `None` while
    /// not connected.
    #[getter]
    fn heartbeat_age(&self) -> Option<f64> {
        seconds(self.0.heartbeat_age)
    }

    /// Round-trip seconds of the latest answered Heartbeat-Request on this
    /// connection; `None` until one is answered (a busy link sends none).
    #[getter]
    fn heartbeat_rtt(&self) -> Option<f64> {
        seconds(self.0.heartbeat_rtt)
    }

    fn __repr__(&self) -> String {
        format!(
            "ScLinkStatus(state='{}', active_hub={})",
            self.state(),
            self.active_hub()
                .map_or_else(|| "None".to_owned(), |hub| format!("'{hub}'"))
        )
    }

    /// Refuses `copy` and `pickle`: a snapshot of a live link. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Async iterator over one subscription to the hub-connector events.
#[pyclass(name = "ScEventIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyScEventIterator(LossyReceiver<ScEvent>);

impl PyScEventIterator {
    pub(crate) fn new(rx: tokio::sync::broadcast::Receiver<ScEvent>) -> Self {
        Self(LossyReceiver::new(rx))
    }
}

#[pymethods]
impl PyScEventIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.0.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(PyScEvent))
                    .ok_or_else(|| crate::event_stream::ended("SC connection events"))
            },
            put_back,
        )
    }

    /// Events this iterator lost: because it fell more than 64 behind, or
    /// dropped on the way to a read, as when its event loop closed first. The
    /// connector never waits for a reader; after a gap, read
    /// `connection_status()` for where the link stands.
    #[getter]
    fn missed(&self) -> u64 {
        self.0.missed()
    }

    /// Refuses `copy` and `pickle`: a live subscription. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyScEvent>()?;
    m.add_class::<PyScLinkStatus>()?;
    m.add_class::<PyScEventIterator>()?;
    Ok(())
}
