//! Sole accepted socket writer with bounded response/control scheduling.
use super::*;
use crate::sc::npdu_admission::DirectPeer;

pub(super) async fn serve_npdu_loop<W>(
    write: &mut W,
    read: &mut W::Read,
    config: &DirectAcceptConfig,
    peer: AdmittedDirectPeer<'_>,
    npdu_tx: &mpsc::Sender<ReceivedNpdu>,
    npdu_admission: &Arc<ScNpduAdmission>,
    responses: &mut mpsc::Receiver<crate::direct_response::ResponseWrite>,
) where
    W: DirectWs,
{
    let AdmittedDirectPeer {
        address: peer_addr,
        member,
        provenance,
        response,
        warnings,
    } = peer;
    let mut retired = member.retirement();
    let mut prefer_send = false;
    let mut idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
    loop {
        if tokio::time::Instant::now() >= idle_deadline {
            debug!("direct idle timeout, closing {peer_addr}");
            let _ = tokio::time::timeout(config.connect_timeout, write.send_close()).await;
            return;
        }
        let next = tokio::select! {
            biased;
            _ = async { if !*retired.borrow_and_update() { let _ = retired.changed().await; } } => {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &crate::sc::direct_membership::disconnect_request());
                let _ = tokio::time::timeout(config.connect_timeout, async {
                    let _ = write.send_data(&buf).await;
                    let _ = write.send_close().await;
                }).await;
                return;
            }
            next = next_event(read, responses, prefer_send, idle_deadline) => next,
        };
        let received = match next {
            Event::Write(Some(request)) => {
                prefer_send = false;
                if !request.can_start() {
                    continue;
                }
                if member.with_current(|| request.mark_started()).is_none() {
                    return;
                }
                let result = tokio::select! {
                    biased;
                    _ = retired.changed() => Err(crate::direct_response::unavailable()),
                    result = tokio::time::timeout(config.connect_timeout, write.send_data(&request.bytes)) => {
                        result.ok().and_then(Result::ok).ok_or_else(crate::direct_response::unavailable)
                    }
                };
                let failed = result.is_err();
                let _ = request.done.send(result);
                // A cancelled/failed partial write is never reused.
                if failed {
                    return;
                }
                // The initiator's AB.6.3 timer restarts on any BVLC it
                // receives, so a peer we keep sending to has no reason to
                // probe; what we send counts as activity too (#2133).
                idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
                continue;
            }
            Event::Write(None) => return,
            Event::Read(result) => {
                prefer_send = true;
                result
            }
        };
        let data = match received {
            Ok(Some(Ok(DirectFrame::Binary(data)))) => data,
            // WebSocket controls are scheduling turns, not BVLC activity.
            Ok(Some(Ok(DirectFrame::Control))) => {
                tokio::task::yield_now().await;
                continue;
            }
            Ok(Some(Err(e))) => {
                crate::warn_throttled!(warnings.frame, "direct recv error from {peer_addr}: {e}");
                return;
            }
            Ok(None) => return,
            Err(_) => {
                debug!("direct idle timeout, closing {peer_addr}");
                let _ = tokio::time::timeout(config.connect_timeout, write.send_close()).await;
                return;
            }
        };
        idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
        if data.len() > config.max_bvlc_length as usize {
            crate::warn_throttled!(
                warnings.frame,
                "direct frame exceeds local Max-BVLC-Length, dropping from {peer_addr}"
            );
            continue;
        }
        let msg = match decode_sc_message(&data) {
            Ok(msg) => msg,
            Err(e) => {
                crate::warn_throttled!(warnings.frame, "direct decode error from {peer_addr}: {e}");
                continue;
            }
        };
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                match direct_must_understand_decision(&msg, &data) {
                    DirectMuDecision::Pass => {}
                    DirectMuDecision::Drop => continue,
                    DirectMuDecision::Nak(nak) => {
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &nak);
                        if !matches!(
                            tokio::time::timeout(config.connect_timeout, write.send_data(&buf))
                                .await,
                            Ok(Ok(()))
                        ) {
                            warn!("direct destination-option NAK send error for {peer_addr}");
                            return;
                        }
                        continue;
                    }
                }
                if let Some(npdu) = direct_npdu(&msg, config) {
                    // Verified direct peer: TLS handshake with operational cert
                    // verified + Connect-Request/Accept completed on this
                    // connection; source_mac is that peer's VMAC. Post-handshake
                    // only; direct connections carry unicast only.
                    member.with_current(|| {
                        npdu_admission.admit_direct_peer(
                            npdu_tx,
                            &msg,
                            npdu,
                            DirectPeer {
                                vmac: member.vmac,
                                addr: peer_addr,
                                provenance: provenance.clone(),
                                response: Some(response.clone()),
                            },
                        )
                    });
                }
            }
            ScFunction::DisconnectRequest => {
                member.retire();
                let ack = ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);
                let _ = tokio::time::timeout(config.connect_timeout, write.send_data(&buf)).await;
                return;
            }
            ScFunction::DisconnectAck => return,
            // AB.6.3: the initiating peer probes an idle connection and the
            // accepting peer answers (#2127). The frame already refreshed
            // the idle deadline above, so a probing initiator keeps the
            // socket open. A malformed probe gets the shared control NAK or
            // silence instead of an ACK.
            ScFunction::HeartbeatRequest => {
                let reply = match crate::sc_frame::validate_control(
                    &msg,
                    &data,
                    crate::sc_frame::ControlRecipient::HubConnector,
                ) {
                    Ok(()) => crate::sc::direct_membership::bare_control(
                        ScFunction::HeartbeatAck,
                        msg.message_id,
                    )
                    .freeze(),
                    Err(Some(nak)) => nak,
                    Err(None) => continue,
                };
                if !matches!(
                    tokio::time::timeout(config.connect_timeout, write.send_data(&reply)).await,
                    Ok(Ok(()))
                ) {
                    warn!("direct heartbeat reply send error for {peer_addr}");
                    return;
                }
            }
            _ => continue,
        }
    }
}

enum Event {
    Read(Result<Option<Result<DirectFrame, String>>, tokio::time::error::Elapsed>),
    Write(Option<crate::direct_response::ResponseWrite>),
}

// A continuously ready input can precede a queued response by at most one
// WebSocket frame (including Ping/Pong); a full response queue can precede
// control by at most one bounded write.
async fn next_event<R: DirectWsRead>(
    read: &mut R,
    responses: &mut mpsc::Receiver<crate::direct_response::ResponseWrite>,
    prefer_send: bool,
    idle_deadline: tokio::time::Instant,
) -> Event {
    if prefer_send {
        tokio::select! {
            biased;
            send = responses.recv() => Event::Write(send),
            read = tokio::time::timeout_at(idle_deadline, read.next_frame()) => Event::Read(read),
        }
    } else {
        tokio::select! {
            biased;
            read = tokio::time::timeout_at(idle_deadline, read.next_frame()) => Event::Read(read),
            send = responses.recv() => Event::Write(send),
        }
    }
}

#[cfg(test)]
#[path = "direct_response_worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "direct_accept_idle_tests.rs"]
mod idle_tests;
