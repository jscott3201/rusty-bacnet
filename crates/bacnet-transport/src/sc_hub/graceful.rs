//! Bounded graceful shutdown for the BACnet/SC hub (RB-13).
//!
//! Forceful [`super::ScHub::stop`] remains unchanged: seal admission and abort
//! workers without any Disconnect exchange. This module implements the
//! protocol-aware alternative: hub-initiated Disconnect-Request per
//! established peer, awaited Disconnect-Ack, then the AB.7.5.5 WebSocket
//! close handshake, all under validated bounds with a forceful fallback.
//!
//! Normative order (licensed Standard 135-2020 PDF, printed pp. 1401-1409):
//!
//! - AB.6.2: both peers run a connect/disconnect state machine. While waiting
//!   for a response after sending Disconnect-Request, a disconnect wait timer
//!   applies; its duration is a local matter. Closing an existing WebSocket
//!   connection before entering IDLE shall be performed per AB.7.5.5.
//! - AB.6.2.3 + Fig. AB-12 (accepting peer): on locally determined
//!   disconnection in CONNECTED, send Disconnect-Request to the initiating
//!   peer, start the disconnect wait timer, enter DISCONNECTING; on
//!   Disconnect-Ack received, close the WebSocket and enter IDLE; on NAK to
//!   the Request, close; on disconnect-wait expiry, close.
//! - AB.7.5.5: the WebSocket close handshake shall be performed when
//!   intentionally closing a connection (RFC 6455), with the close-status to
//!   error-code map.
//!
//! Corrected role wording (official 2024-04-29 errata summary, item 13,
//! Annex AB.6.2.3 p. 1405, visually verified redline on errata PDF p. 4):
//!
//! - Title: "Disconnecting-ACK message should be from the initiating peer."
//! - Redline direction: the italic word "initiating" is inserted and the
//!   struck-through word "accepting" is removed, so the corrected
//!   DISCONNECTING text reads: "On receipt of a Disconnect-ACK message from
//!   the initiating peer, close the WebSocket connection, and enter the IDLE
//!   state." The supplied base PDF still shows the pre-errata "from the
//!   accepting peer" in AB.6.2.3; this implementation follows the corrected
//!   "from the initiating peer" direction.
//!
//! Adopted close order per established peer: snapshot-then-release the
//! registry (never hold the Clients map or sink lock across waits), send a
//! hub-initiated Disconnect-Request (no VMACs/options/payload, per-connection
//! message ID 0; the function code distinguishes it from hub HeartbeatRequest
//! IDs), await the matching Disconnect-Ack with the per-peer ack bound, then
//! perform the AB.7.5.5 close handshake (send Close, await peer Close echo
//! with the per-close bound). Half-handshakes (accepted but unregistered)
//! get a silent Close only, never a Disconnect-Request: there is no
//! established BACnet/SC connection to disconnect. The overall bound caps the
//! whole drain; on expiry the supervisor falls back to forceful abort.
//!
//! Outcome discipline: only an Ack-observed close counts as graceful. Any
//! per-peer ack/close timeout, peer-closes-first without Ack, send failure
//! after the Request was issued, NAK to the Request, or supervisor abort
//! marks the run forced. A task ending during same-UUID replacement must not
//! remove the new registration (existing ptr-eq guards in
//! [`super::retirement`] and [`super::relay_send`] still own removal) and a
//! pre-Request supersede exits quietly so the replacement task can drive its
//! own exchange; a supersede after the Request was sent marks forced
//! (conservative: never claim clean graceful when a Request went
//! unacknowledged). The closed predicate wins: an already-closed peer is
//! skipped without I/O.
//!
//! Status note: closing-but-still-registered peers remain in `client_count`
//! until their lease cleanup removes them. There is no separate closing
//! count; that would double-count the same registration.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{watch, Mutex, Notify};
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;

use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};

use super::{Clients, DeviceUuid, Vmac, WsSink};

/// Hub-initiated Disconnect message ID (per-connection scope).
///
/// The function code (0x08/0x09) distinguishes this exchange from hub
/// HeartbeatRequest IDs (0x8000+) on the same connection, so a fixed low ID
/// is safe without a shared allocator.
const GRACEFUL_DISCONNECT_ID: u16 = 0;

/// Validated bounds for one graceful shutdown run.
///
/// - `disconnect_ack`: per-peer budget for sending Disconnect-Request
///   (including sink acquisition) and awaiting the matching Disconnect-Ack.
///   Local matter per AB.6.2; seconds-scale.
/// - `ws_close`: per-peer budget for the AB.7.5.5 close handshake (send
///   Close, await peer Close echo). Seconds-scale.
/// - `overall`: bound for the whole drain across all peers run in parallel.
///   Must cover at least one ack + one close phase; startup rejects an
///   overall smaller than that sum. Seconds-scale.
///
/// All values are validated before any socket opens (via
/// [`super::ScHubTlsConfig::with_graceful_timeouts`] at startup) and again
/// by [`Self::new`]. Invalid bounds return [`Error::Encoding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubGracefulTimeouts {
    disconnect_ack: Duration,
    ws_close: Duration,
    overall: Duration,
}

impl ScHubGracefulTimeouts {
    /// Construct checked graceful bounds.
    pub fn new(
        disconnect_ack: Duration,
        ws_close: Duration,
        overall: Duration,
    ) -> Result<Self, Error> {
        for (name, value, minimum) in [
            ("Disconnect-Ack", disconnect_ack, Duration::from_secs(1)),
            ("WebSocket close", ws_close, Duration::from_secs(1)),
            ("Graceful overall", overall, Duration::from_secs(2)),
        ] {
            if value < minimum || value > Duration::from_secs(300) {
                return Err(Error::Encoding(format!(
                    "Hub {name} timeout must be between {minimum:?} and 300s"
                )));
            }
        }
        if overall < disconnect_ack.saturating_add(ws_close) {
            return Err(Error::Encoding(format!(
                "Hub graceful overall timeout ({overall:?}) must cover ack ({disconnect_ack:?}) + close ({ws_close:?})"
            )));
        }
        Ok(Self {
            disconnect_ack,
            ws_close,
            overall,
        })
    }

    /// Per-peer Disconnect-Request send + Disconnect-Ack wait budget.
    pub fn disconnect_ack(self) -> Duration {
        self.disconnect_ack
    }

    /// Per-peer AB.7.5.5 close-handshake budget.
    pub fn ws_close(self) -> Duration {
        self.ws_close
    }

    /// Whole-drain bound across parallel peers.
    pub fn overall(self) -> Duration {
        self.overall
    }

    pub(super) fn validate(self) -> Result<(), Error> {
        Self::new(self.disconnect_ack, self.ws_close, self.overall).map(|_| ())
    }
}

impl Default for ScHubGracefulTimeouts {
    fn default() -> Self {
        Self {
            disconnect_ack: Duration::from_secs(5),
            ws_close: Duration::from_secs(5),
            overall: Duration::from_secs(15),
        }
    }
}

/// Outcome of [`super::ScHub::shutdown_gracefully`].
///
/// `Graceful` means every established peer observed its Disconnect-Ack and
/// completed the WebSocket close handshake, half-handshakes closed silently,
/// and no per-peer timeout or supervisor abort occurred. `Forced` means
/// bounded timeout or forced cleanup happened somewhere: silent peer,
/// peer-closes-first without Ack, NAK to the Request, close-echo timeout, a
/// supersede after the Request was sent, overall expiry with abort, or a
/// prior forceful [`super::ScHub::stop`]. A task abort is never reported as
/// graceful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScHubShutdownOutcome {
    /// Protocol-aware close completed for all peers within bounds.
    Graceful,
    /// Timeout or forced cleanup occurred; see module docs.
    Forced,
}

/// Shared graceful context for one hub.
///
/// Cloned into every accepted connection task at spawn. The watch fires once
/// per hub lifetime (first graceful seal wins; a prior forceful seal keeps
/// the run forced). `failed` is set on any per-peer timeout or
/// unacknowledged close; the accept loop combines it with overall-expiry to
/// publish the outcome. Timeouts are immutable per hub.
#[derive(Clone)]
pub(super) struct GracefulCtx {
    signal: watch::Receiver<bool>,
    // Sender clone keeps the channel open for the task's lifetime, so
    // `wait_for` never reports a dropped channel even when the originating
    // supervisor value was temporary (direct test harnesses). Signaling
    // still flows from the hub-owned supervisor through the shared channel.
    _keepalive: watch::Sender<bool>,
    failed: Arc<AtomicBool>,
    timeouts: ScHubGracefulTimeouts,
}

impl GracefulCtx {
    pub(super) fn new(
        sender: watch::Sender<bool>,
        failed: Arc<AtomicBool>,
        timeouts: ScHubGracefulTimeouts,
    ) -> Self {
        Self {
            signal: sender.subscribe(),
            _keepalive: sender,
            failed,
            timeouts,
        }
    }

    pub(super) fn signal(&self) -> watch::Receiver<bool> {
        self.signal.clone()
    }

    pub(super) fn timeouts(&self) -> ScHubGracefulTimeouts {
        self.timeouts
    }

    pub(super) fn note_failed(&self) {
        self.failed.store(true, Ordering::Release);
    }
}

/// Poll one inbound frame or the graceful signal (biased toward shutdown).
///
/// Returns `None` when graceful was signaled before the next frame was
/// ready; the caller then runs [`exchange`] and breaks. Otherwise returns
/// the read outcome for normal dispatch. Never holds map/sink locks.
pub(super) async fn next_or_graceful(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<super::TlsStream>,
    >,
    signal: &mut watch::Receiver<bool>,
) -> Option<Option<Result<Message, tokio_tungstenite::tungstenite::Error>>> {
    tokio::select! {
        biased;
        result = signal.wait_for(|requested| *requested) => {
            let _ = result;
            None
        }
        message = read.next() => Some(message),
    }
}

/// Run the graceful exchange for one connection task, then return.
///
/// - `lease_vmac` `None` (half-handshake): silent Close only, never
///   Disconnect-Request.
/// - `Some(vmac)` (established): entry revalidates under a brief map lock
///   (ptr-eq + not closed); a pre-Request supersede exits quietly so a
///   replacement task can drive its own exchange. After the Request is sent,
///   any missing Ack (timeout, peer Request/Close first, NAK, send failure,
///   close-echo timeout, supersede) marks the run forced.
///
/// Always ends the connection; the caller breaks to the existing lease
/// cleanup (ptr-eq-guarded removal + bounded Close, harmlessly repeated).
/// Never holds the Clients lock across waits.
#[allow(clippy::too_many_arguments)]
pub(super) async fn exchange(
    peer_addr: SocketAddr,
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<super::TlsStream>,
    >,
    write: &Arc<Mutex<WsSink>>,
    clients: &Clients,
    lease_vmac: Option<Vmac>,
    closed: &Arc<AtomicBool>,
    notify: &Arc<Notify>,
    ctx: &GracefulCtx,
) {
    let Some(vmac) = lease_vmac else {
        silent_close(peer_addr, read, write, ctx).await;
        return;
    };
    // Snapshot-then-release: brief lock, clone identity, release before I/O.
    let device_uuid: Option<DeviceUuid> = {
        let map = clients.lock().await;
        match map.get(&vmac) {
            Some(client)
                if Arc::ptr_eq(&client.sink, write) && !client.closed.load(Ordering::Acquire) =>
            {
                Some(client.device_uuid)
            }
            _ => None,
        }
    };
    let Some(_device_uuid) = device_uuid else {
        // Already replaced/retired before the Request: closed wins quietly.
        debug!("Hub graceful: {peer_addr} already retired, skipping DisconnectRequest");
        return;
    };
    if closed.load(Ordering::Acquire) {
        debug!("Hub graceful: {peer_addr} closed before DisconnectRequest");
        return;
    }
    if !send_disconnect_request(peer_addr, write, ctx).await {
        ctx.note_failed();
        return;
    }
    match await_ack(peer_addr, read, write, clients, vmac, closed, notify, ctx).await {
        AckOutcome::Acked => {
            if !close_handshake(peer_addr, read, write, ctx).await {
                ctx.note_failed();
            }
        }
        AckOutcome::QuietSuperseded => {}
        AckOutcome::Failed => ctx.note_failed(),
    }
}

enum AckOutcome {
    Acked,
    QuietSuperseded,
    Failed,
}

async fn send_disconnect_request(
    peer_addr: SocketAddr,
    write: &Arc<Mutex<WsSink>>,
    ctx: &GracefulCtx,
) -> bool {
    let request = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: GRACEFUL_DISCONNECT_ID,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    let frame = Message::Binary(buf.freeze());
    // Timeout covers sink acquisition (a held sink must not stall the drain)
    // plus the send itself; only sink -> Clients nesting is permitted and no
    // Clients lock is held here.
    let result = tokio::time::timeout(ctx.timeouts.disconnect_ack(), async {
        let mut sink = write.lock().await;
        sink.send(frame).await
    })
    .await;
    match result {
        Ok(Ok(())) => {
            debug!("Hub graceful: DisconnectRequest sent to {peer_addr}");
            true
        }
        Ok(Err(e)) => {
            debug!("Hub graceful: DisconnectRequest send to {peer_addr} failed: {e}");
            false
        }
        Err(_) => {
            debug!("Hub graceful: DisconnectRequest send to {peer_addr} timed out");
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn await_ack(
    peer_addr: SocketAddr,
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<super::TlsStream>,
    >,
    write: &Arc<Mutex<WsSink>>,
    clients: &Clients,
    vmac: Vmac,
    closed: &Arc<AtomicBool>,
    notify: &Arc<Notify>,
    ctx: &GracefulCtx,
) -> AckOutcome {
    let wait = async {
        loop {
            if closed.load(Ordering::Acquire) {
                // Replacement (same UUID re-registered elsewhere) exits
                // quietly only when decided before the Request; after the
                // Request a supersede still counts as unacknowledged
                // (conservative: never claim graceful without the Ack).
                // The ptr-eq recheck below keeps removal attribution exact.
                let replaced = {
                    let map = clients.lock().await;
                    match map.get(&vmac) {
                        Some(client) => !Arc::ptr_eq(&client.sink, write),
                        None => true,
                    }
                };
                let _ = replaced;
                return AckOutcome::Failed;
            }
            tokio::select! {
                biased;
                _ = super::retirement::wait(closed, notify) => {
                    return AckOutcome::Failed;
                }
                message = read.next() => {
                    match message {
                        None => return AckOutcome::Failed,
                        Some(Err(_)) => return AckOutcome::Failed,
                        Some(Ok(Message::Close(_))) => {
                            debug!("Hub graceful: {peer_addr} closed before Ack");
                            return AckOutcome::Failed;
                        }
                        Some(Ok(Message::Binary(data))) => {
                            let decoded = match decode_sc_message(&data) {
                                Ok(m) => m,
                                Err(_) => continue,
                            };
                            match decoded.function {
                                ScFunction::DisconnectAck
                                    if decoded.message_id == GRACEFUL_DISCONNECT_ID =>
                                {
                                    // Envelope faults on a response are
                                    // ignored (responses never get NAKs);
                                    // only a clean Ack for our ID counts.
                                    if decoded.originating_vmac.is_none()
                                        && decoded.destination_vmac.is_none()
                                        && decoded.data_options.is_empty()
                                        && decoded.payload.is_empty()
                                    {
                                        debug!("Hub graceful: Ack observed from {peer_addr}");
                                        return AckOutcome::Acked;
                                    }
                                }
                                ScFunction::DisconnectRequest => {
                                    // Simultaneous disconnect: answer the
                                    // peer's Request (existing accepting
                                    // behavior) but our own Request went
                                    // unacknowledged.
                                    let ack = ScMessage {
                                        function: ScFunction::DisconnectAck,
                                        message_id: decoded.message_id,
                                        originating_vmac: None,
                                        destination_vmac: None,
                                        dest_options: Vec::new(),
                                        data_options: Vec::new(),
                                        payload: Bytes::new(),
                                    };
                                    let mut buf = BytesMut::new();
                                    encode_sc_message(&mut buf, &ack);
                                    let _ = tokio::time::timeout(
                                        ctx.timeouts.ws_close(),
                                        async {
                                            let mut sink = write.lock().await;
                                            sink.send(Message::Binary(buf.freeze())).await
                                        },
                                    )
                                    .await;
                                    debug!("Hub graceful: {peer_addr} requested first, closing");
                                    return AckOutcome::Failed;
                                }
                                ScFunction::Result => {
                                    if is_nak_for_disconnect(&decoded) {
                                        debug!("Hub graceful: NAK to Disconnect from {peer_addr}");
                                        return AckOutcome::Failed;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(_)) => {}
                    }
                }
            }
        }
    };
    match tokio::time::timeout(ctx.timeouts.disconnect_ack(), wait).await {
        Ok(outcome) => outcome,
        Err(_) => {
            debug!("Hub graceful: Ack wait for {peer_addr} timed out");
            AckOutcome::Failed
        }
    }
}

fn is_nak_for_disconnect(msg: &ScMessage) -> bool {
    if msg.function != ScFunction::Result || msg.message_id != GRACEFUL_DISCONNECT_ID {
        return false;
    }
    match crate::sc_frame::decode_sc_bvlc_result(msg) {
        Ok(crate::sc_frame::ScBvlcResult::Nak { result_for, .. }) => {
            result_for == ScFunction::DisconnectRequest
        }
        _ => false,
    }
}

/// AB.7.5.5 close handshake after an observed Ack: send Close, await the
/// peer Close echo within the per-close bound. Returns true on echo.
async fn close_handshake(
    peer_addr: SocketAddr,
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<super::TlsStream>,
    >,
    write: &Arc<Mutex<WsSink>>,
    ctx: &GracefulCtx,
) -> bool {
    let send = tokio::time::timeout(ctx.timeouts.ws_close(), async {
        let mut sink = write.lock().await;
        sink.send(Message::Close(None)).await
    })
    .await;
    if !matches!(send, Ok(Ok(()))) {
        debug!("Hub graceful: Close send to {peer_addr} failed");
        return false;
    }
    let echo = tokio::time::timeout(ctx.timeouts.ws_close(), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return false,
            }
        }
    })
    .await;
    matches!(echo, Ok(true))
}

/// Silent Close for half-handshakes: never a Disconnect-Request.
async fn silent_close(
    peer_addr: SocketAddr,
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<super::TlsStream>,
    >,
    write: &Arc<Mutex<WsSink>>,
    ctx: &GracefulCtx,
) {
    let send = tokio::time::timeout(ctx.timeouts.ws_close(), async {
        let mut sink = write.lock().await;
        sink.send(Message::Close(None)).await
    })
    .await;
    if !matches!(send, Ok(Ok(()))) {
        debug!("Hub graceful: silent Close to half-handshake {peer_addr} failed");
        return;
    }
    let _ = tokio::time::timeout(ctx.timeouts.ws_close(), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    debug!("Hub graceful: half-handshake {peer_addr} closed silently");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_ack_plus_close_within_overall() {
        let timeouts = ScHubGracefulTimeouts::default();
        assert_eq!(timeouts.disconnect_ack(), Duration::from_secs(5));
        assert_eq!(timeouts.ws_close(), Duration::from_secs(5));
        assert_eq!(timeouts.overall(), Duration::from_secs(15));
        assert!(timeouts.overall() >= timeouts.disconnect_ack() + timeouts.ws_close());
    }

    #[test]
    fn checked_constructor_rejects_subsecond_and_short_overall() {
        for (ack, close, overall) in [
            (
                Duration::from_millis(999),
                Duration::from_secs(5),
                Duration::from_secs(15),
            ),
            (
                Duration::from_secs(5),
                Duration::from_millis(999),
                Duration::from_secs(15),
            ),
            (
                Duration::from_secs(5),
                Duration::from_secs(5),
                Duration::from_secs(9),
            ),
            (
                Duration::from_secs(5),
                Duration::from_secs(5),
                Duration::from_secs(301),
            ),
        ] {
            assert!(
                ScHubGracefulTimeouts::new(ack, close, overall).is_err(),
                "{ack:?}/{close:?}/{overall:?}"
            );
        }
        assert!(ScHubGracefulTimeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .is_ok());
    }

    #[test]
    fn outcome_distinguishes_graceful_from_forced() {
        assert_ne!(ScHubShutdownOutcome::Graceful, ScHubShutdownOutcome::Forced);
        assert_eq!(format!("{:?}", ScHubShutdownOutcome::Graceful), "Graceful");
    }
}
