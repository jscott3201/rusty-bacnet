//! Hub-connector observability as data (#959): which hub is active, a
//! structured event stream, and a link-status snapshot with heartbeat timing.
//!
//! The tracing lines stay; these mirror them for programs such as a TUI. An
//! event is sent with a synchronous broadcast send, so the connector never
//! waits for a subscriber: one that falls more than [`SC_EVENT_CAPACITY`]
//! events behind gets `RecvError::Lagged` and the oldest events are lost.

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::time::Instant;

use crate::sc_frame::{decode_sc_bvlc_result, ScBvlcResult, ScFunction, ScMessage, Vmac};

use super::{
    ScConnectError, ScConnection, ScConnectionState, ScDialStage, ScTlsFailure, ScTransport,
    ScWebSocketErrorKind, WebSocketPort,
};

/// Events a subscriber can fall behind by before it starts losing the oldest.
pub const SC_EVENT_CAPACITY: usize = 64;

/// Which of the two configured hubs a hub connection is to (Annex AB.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScActiveHub {
    /// The primary hub.
    Primary,
    /// The failover hub, used while the primary cannot be reached.
    Failover,
}

/// Why a hub connection ended or a connect attempt failed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScDisconnectCause {
    /// This node closed the connection: the transport was stopped or aborted.
    LocalClose,
    /// The hub closed it: a Disconnect-Request from the hub, or the
    /// WebSocket closed or ended.
    PeerClose,
    /// Nothing arrived from the hub within the heartbeat timeout. Annex
    /// AB.6.3 has the connecting node probe an idle link with Heartbeat-Request;
    /// dropping the connection when the silence outlasts the timeout is this
    /// stack's local policy.
    HeartbeatTimeout,
    /// A TCP, TLS or WebSocket failure of the given kind.
    WebSocket(ScWebSocketErrorKind),
    /// A TLS failure the TLS library explained: a certificate this node
    /// refused, or an alert from the hub refusing this node (Annex AB.7.4).
    Tls(ScTlsFailure),
    /// The hub answered with a BVLC-Result NAK: to the Connect-Request while
    /// connecting, or a NAK that ends a live connection (Annex AB.6).
    #[non_exhaustive]
    Nak {
        /// The BVLC function the NAK answers.
        result_for: ScFunction,
        /// Error class carried by the NAK.
        error_class: ErrorClass,
        /// Error code carried by the NAK.
        error_code: ErrorCode,
    },
    /// The hub dial (host lookup, TCP, TLS and WebSocket upgrade) did not
    /// finish within the connect timeout (#1839).
    #[non_exhaustive]
    DialTimeout {
        /// The step the dial was in, when the dialler reports it.
        stage: Option<ScDialStage>,
    },
    /// The WebSocket opened, but no Connect-Accept answered the
    /// Connect-Request within the connect timeout (#1839).
    ConnectAcceptTimeout,
    /// The server answered the WebSocket upgrade with this HTTP status
    /// instead of switching protocols: often a proxy or load balancer in
    /// the path, such as a 502 from one that cannot reach the hub (#1839).
    #[non_exhaustive]
    UpgradeRefused {
        /// The HTTP status code.
        http_status: u16,
    },
    /// A BVLC-Result that could not be decoded.
    MalformedResult,
    /// A reply to the Connect-Request that does not complete it: a
    /// mismatched Connect-Accept or a BVLC-Result ACK.
    UnexpectedConnectReply,
    /// A rejection NAK could not be sent within the remaining heartbeat
    /// budget, so the socket was retired.
    RejectionExpired,
    /// Any other failure, with its text. The text is for diagnostics only:
    /// it is not stable and should not be matched on.
    Other(String),
}

impl ScDisconnectCause {
    /// Classify a connect-path or WebSocket error.
    ///
    /// A bare [`Error::Timeout`] reads as [`Self::ConnectAcceptTimeout`],
    /// since a dial run through [`dial_within`](super::dial_within) times out
    /// as [`ScConnectError::DialTimeout`]. A custom connector that returns
    /// `Error::Timeout` itself for a slow dial is therefore reported as a
    /// Connect-Accept timeout.
    pub fn from_error(error: &Error) -> Self {
        // A dial's timeout is typed; the bare timeout is the Connect-Request's.
        if let Error::Timeout(_) = error {
            return Self::ConnectAcceptTimeout;
        }
        match ScConnectError::from_error(error) {
            Some(ScConnectError::HandshakeNak {
                result_for,
                error_class,
                error_code,
                ..
            }) => Self::Nak {
                result_for: *result_for,
                error_class: ErrorClass::from_raw(*error_class),
                error_code: ErrorCode::from_raw(*error_code),
            },
            Some(ScConnectError::MalformedBvlcResult { .. }) => Self::MalformedResult,
            Some(
                ScConnectError::UnexpectedResultAck { .. } | ScConnectError::ConnectAcceptMismatch,
            ) => Self::UnexpectedConnectReply,
            Some(ScConnectError::DialTimeout { stage, .. }) => Self::DialTimeout { stage: *stage },
            Some(ScConnectError::WebSocket { tls: Some(tls), .. }) => Self::Tls(*tls),
            Some(ScConnectError::WebSocket {
                http_status: Some(http_status),
                ..
            }) => Self::UpgradeRefused {
                http_status: *http_status,
            },
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::Closed | ScWebSocketErrorKind::StreamEnded,
                ..
            }) => Self::PeerClose,
            Some(ScConnectError::WebSocket { kind, .. }) => Self::WebSocket(*kind),
            None => Self::Other(error.to_string()),
        }
    }

    /// The cause when a received message moved a live connection to
    /// Disconnected.
    fn from_received(msg: &ScMessage) -> Self {
        match msg.function {
            ScFunction::DisconnectRequest => Self::PeerClose,
            // Completes a Disconnect-Request this node sent.
            ScFunction::DisconnectAck => Self::LocalClose,
            _ => match decode_sc_bvlc_result(msg) {
                // Even refused, our Disconnect-Request ends the connection.
                Ok(ScBvlcResult::Nak {
                    result_for: ScFunction::DisconnectRequest,
                    ..
                }) => Self::LocalClose,
                Ok(ScBvlcResult::Nak {
                    result_for,
                    error_class,
                    error_code,
                    ..
                }) => Self::Nak {
                    result_for,
                    error_class: ErrorClass::from_raw(error_class),
                    error_code: ErrorCode::from_raw(error_code),
                },
                _ => Self::MalformedResult,
            },
        }
    }
}

/// A hub-connector event, in the order the connector acts.
///
/// - A failover reads `Disconnected`, then `ReconnectAttempt` and
///   `ConnectFailed` for each failed primary attempt, then `FailoverEngaged`
///   and `Connected` on the failover hub.
/// - Losing the failover hub retries the primary first, then the failover
///   hub, within each attempt; both share the attempt number.
/// - A primary restore reads `PrimaryRestored`, then `Connected` on the
///   primary. The replaced failover connection gets no `Disconnected`.
/// - While on the failover hub, each restore probe of the primary that
///   fails reads `PrimaryRestoreFailed`: at most one per restore interval,
///   since the next probe waits a full interval after this one ends.
/// - A stop during a reconnect backoff ends the stream without a terminal
///   event: the last one is `Disconnected` or the latest `ConnectFailed`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScEvent {
    /// A hub connection was established and is now the one in use.
    #[non_exhaustive]
    Connected {
        /// The hub it is to.
        hub: ScActiveHub,
        /// The hub's VMAC from its Connect-Accept, which names the device
        /// hosting the hub function (Annex AB.5.3.1).
        hub_vmac: Vmac,
    },
    /// The hub connection in use ended.
    #[non_exhaustive]
    Disconnected {
        /// The hub it was to.
        hub: ScActiveHub,
        /// Why it ended.
        cause: ScDisconnectCause,
    },
    /// A connect attempt failed before the connection was established.
    #[non_exhaustive]
    ConnectFailed {
        /// The hub being dialled.
        hub: ScActiveHub,
        /// Why it failed.
        cause: ScDisconnectCause,
    },
    /// A reconnect attempt is dialling `hub`, after waiting `backoff`.
    #[non_exhaustive]
    ReconnectAttempt {
        /// The hub being dialled.
        hub: ScActiveHub,
        /// The attempt number, from 1.
        attempt: u32,
        /// The jittered delay waited before this attempt.
        backoff: Duration,
    },
    /// The primary hub could not be reached; the connector turns to the
    /// failover hub (Annex AB.5.2).
    FailoverEngaged,
    /// The primary hub is back and has taken over from the failover hub.
    PrimaryRestored,
    /// A probe of the primary hub, made while on the failover hub, failed;
    /// the failover connection stays in use (#1828).
    #[non_exhaustive]
    PrimaryRestoreFailed {
        /// Why the probe failed, sorted as for `ConnectFailed`.
        cause: ScDisconnectCause,
    },
    /// Reconnection stopped; the transport stays disconnected.
    #[non_exhaustive]
    ReconnectGaveUp {
        /// Attempts made in this episode, the highest `attempt` number
        /// reported. The single failover dial after the primary attempts
        /// are used up is not counted.
        attempts: u32,
    },
}

/// A snapshot of the hub link, from [`ScTransport::connection_status`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScLinkStatus {
    /// The connection state.
    pub state: ScConnectionState,
    /// The hub of the connection in use or, while reconnecting, the hub
    /// being dialled; `None` before the first connect attempt.
    pub active_hub: Option<ScActiveHub>,
    /// The connected hub's VMAC from Connect-Accept; `None` when not connected.
    pub hub_vmac: Option<Vmac>,
    /// Time since the last message accepted from the hub, the idle time
    /// the heartbeat timer measures (Annex AB.6.3); `None` when not connected.
    pub heartbeat_age: Option<Duration>,
    /// Round-trip time of the latest answered Heartbeat-Request on this
    /// connection. Heartbeats go out only on an idle link, so a busy link
    /// may have none.
    pub heartbeat_rtt: Option<Duration>,
}

#[derive(Default)]
struct Link {
    connected: bool,
    hub: Option<ScActiveHub>,
    hub_vmac: Option<Vmac>,
    last_activity: Option<Instant>,
    heartbeat_sent: Option<Instant>,
    heartbeat_rtt: Option<Duration>,
}

/// Event sender and link bookkeeping, shared by the transport and its
/// receive loop. The link mutex is a leaf: nothing else is locked or
/// awaited while it is held.
pub(super) struct ScObserver {
    events: broadcast::Sender<ScEvent>,
    link: StdMutex<Link>,
}

impl ScObserver {
    pub(super) fn new() -> Self {
        Self {
            events: broadcast::channel(SC_EVENT_CAPACITY).0,
            link: StdMutex::new(Link::default()),
        }
    }

    fn link(&self) -> std::sync::MutexGuard<'_, Link> {
        self.link.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: ScEvent) {
        // No receivers is not an error worth reporting.
        let _ = self.events.send(event);
    }

    /// Mark the link connected and report it. A handshake always learns the
    /// hub VMAC, so without one neither happens: the flag and the event
    /// stay paired.
    pub(super) fn connected(&self, hub: ScActiveHub, hub_vmac: Option<Vmac>) {
        let Some(hub_vmac) = hub_vmac else {
            return;
        };
        {
            let mut link = self.link();
            *link = Link {
                connected: true,
                hub: Some(hub),
                hub_vmac: Some(hub_vmac),
                last_activity: Some(Instant::now()),
                ..Link::default()
            };
        }
        self.emit(ScEvent::Connected { hub, hub_vmac });
    }

    /// Report the live connection's end once, whichever path sees it first.
    /// Always clears the per-connection fields, so a late call (stop after a
    /// reconnect that raced it) leaves a consistent status.
    pub(super) fn disconnected(&self, cause: ScDisconnectCause) {
        let (was_connected, hub) = {
            let mut link = self.link();
            let was_connected = link.connected;
            let hub = link.hub;
            *link = Link {
                hub,
                ..Link::default()
            };
            (was_connected, hub)
        };
        if let (true, Some(hub)) = (was_connected, hub) {
            self.emit(ScEvent::Disconnected { hub, cause });
        }
    }

    pub(super) fn disconnected_by(&self, msg: &ScMessage) {
        self.disconnected(ScDisconnectCause::from_received(msg));
    }

    /// Mark the live connection Disconnected, then report why.
    pub(super) async fn lose(
        &self,
        conn: &Mutex<ScConnection>,
        state_tx: &watch::Sender<ScConnectionState>,
        cause: ScDisconnectCause,
    ) {
        {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            state_tx.send_replace(c.state);
        }
        self.disconnected(cause);
    }

    pub(super) fn connect_failed(&self, hub: ScActiveHub, error: &Error) {
        self.emit(ScEvent::ConnectFailed {
            hub,
            cause: ScDisconnectCause::from_error(error),
        });
    }

    pub(super) fn reconnect_attempt(&self, hub: ScActiveHub, attempt: u32, backoff: Duration) {
        self.link().hub = Some(hub);
        self.emit(ScEvent::ReconnectAttempt {
            hub,
            attempt,
            backoff,
        });
    }

    pub(super) fn failover_engaged(&self) {
        self.link().hub = Some(ScActiveHub::Failover);
        self.emit(ScEvent::FailoverEngaged);
    }

    pub(super) fn primary_restored(&self) {
        self.emit(ScEvent::PrimaryRestored);
    }

    pub(super) fn primary_restore_failed(&self, error: &Error) {
        self.emit(ScEvent::PrimaryRestoreFailed {
            cause: ScDisconnectCause::from_error(error),
        });
    }

    pub(super) fn gave_up(&self, attempts: u32) {
        self.emit(ScEvent::ReconnectGaveUp { attempts });
    }

    /// A message from the hub was accepted: the heartbeat idle time restarts.
    pub(super) fn activity(&self) {
        self.link().last_activity = Some(Instant::now());
    }

    pub(super) fn heartbeat_sent(&self) {
        self.link().heartbeat_sent = Some(Instant::now());
    }

    /// The outstanding Heartbeat-Request was answered.
    pub(super) fn heartbeat_acked(&self) {
        let now = Instant::now();
        let mut link = self.link();
        link.last_activity = Some(now);
        if let Some(sent) = link.heartbeat_sent.take() {
            link.heartbeat_rtt = Some(now.saturating_duration_since(sent));
        }
    }

    fn status(&self, state: ScConnectionState) -> ScLinkStatus {
        let link = self.link();
        ScLinkStatus {
            state,
            active_hub: link.hub,
            hub_vmac: link.hub_vmac,
            heartbeat_age: link.last_activity.map(|at| at.elapsed()),
            heartbeat_rtt: link.heartbeat_rtt,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<ScEvent> {
        self.events.subscribe()
    }
}

impl<W: WebSocketPort> ScTransport<W> {
    /// Subscribe to hub-connector events: connects, disconnects with their
    /// cause, reconnect attempts, failover and primary restore.
    ///
    /// Subscribe before [`start`](crate::port::TransportPort::start) to see
    /// the first connection. Unlike
    /// [`connection_state_changes`](Self::connection_state_changes), events
    /// do not coalesce, but a receiver more than [`SC_EVENT_CAPACITY`] events
    /// behind loses the oldest (`RecvError::Lagged`); the connector never
    /// waits for it.
    pub fn connection_events(&self) -> broadcast::Receiver<ScEvent> {
        self.observer.subscribe()
    }

    /// A snapshot of the hub link: state, active hub, hub VMAC and heartbeat
    /// timing. It never waits on the connection lock or the receive loop, so
    /// it can be polled as often as a UI redraws.
    pub fn connection_status(&self) -> ScLinkStatus {
        self.observer.status(*self.state_tx.borrow())
    }
}

#[cfg(test)]
#[path = "observability_tests.rs"]
mod tests;
