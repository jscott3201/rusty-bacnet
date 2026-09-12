//! Opaque relay mechanics only. The caller owns explicit family admission.
use super::*;

pub(super) async fn relay(
    wire: &[u8],
    msg: &ScMessage,
    source: Vmac,
    target: HubRelayTarget,
    clients: &Clients,
    source_sink: &Arc<Mutex<WsSink>>,
) -> ResultRelayDisposition {
    let Some(frame) = encode_hub_relay_frame(wire, msg, source, target) else {
        return ResultRelayDisposition::Continue;
    };
    let frame = frame.freeze();
    let sinks: Vec<HubRelaySink> = {
        let map = clients.lock().await;
        if !registered_client_matches_sink_in_map(&map, source, source_sink) {
            return ResultRelayDisposition::CloseSource;
        }
        hub_relay_recipient_vmacs(target, source, map.keys().copied())
            .into_iter()
            .filter(|vmac| *vmac != source)
            .filter_map(|vmac| {
                let client = map.get(&vmac)?;
                // The body is opaque, not an NPDU. Only the final encoded BVLC
                // length applies, including the added broadcast origin address.
                if frame.len() > usize::from(client.max_bvlc) {
                    return None;
                }
                Some(HubRelaySink::capture(vmac, client))
            })
            .collect()
    };
    let sends = sinks.into_iter().map(|sink| {
        let frame = frame.clone();
        async move {
            let send = super::relay_send::send(
                &sink,
                clients,
                Message::Binary(frame),
                &super::relay_send::SocketIo,
            );
            // Preserve NPDU relay ownership and waits: parallel, individually
            // bounded broadcast; unicast interrupted by source/target retirement.
            if target == HubRelayTarget::Broadcast {
                if let Err(_) | Ok(Err(_)) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), send).await
                {
                    warn!("Hub: opaque broadcast relay failed to {:02x?}", sink.vmac);
                }
            } else if let Err(error) = send.await {
                warn!(
                    "Hub: opaque unicast relay failed to {:02x?}: {error}",
                    sink.vmac
                );
            }
        }
    });
    futures_util::future::join_all(sends).await;
    ResultRelayDisposition::Continue
}
