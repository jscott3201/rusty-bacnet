use std::sync::Arc;

use bytes::BytesMut;

use bacnet_types::error::{Error, Lifecycle, Refusal};

use crate::port::DataAttribute;
use crate::sc_frame::{BROADCAST_VMAC, Vmac, encode_sc_message};

use super::{ScConnectionState, ScTransport, WebSocketPort};

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) async fn send_unicast_inner(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        if self.ws_shared.is_none() {
            return Err(not_started(&self.stop_gate));
        }
        // Aborted (#2617): nothing more goes out, whether or not the hub
        // socket has left its slot yet.
        if self.abort_mark.is_set() {
            return Err(crate::port::stopped_for_good());
        }
        if mac.len() != 6 {
            return Err(Error::InvalidArgument(
                Refusal::new("BACnet/SC VMAC must be 6 bytes, got {}").with_value(mac.len() as u64),
            ));
        }
        let mut dest_vmac = [0u8; 6];
        dest_vmac.copy_from_slice(mac);

        if dest_vmac != BROADCAST_VMAC
            && let Some(route) = self.direct_membership.route(&dest_vmac)
        {
            match route.send_npdu(npdu, data_attributes).await {
                Ok(()) => return Ok(()),
                Err(super::direct_egress::DirectSendError::Unavailable) => {}
                Err(error) => return Err(error.into_error()),
            }
        }

        let Some(direct) = self.direct_shared().filter(|_| dest_vmac != BROADCAST_VMAC) else {
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        };
        let (Some(ws_shared), Some(conn)) = (&self.ws_shared, &self.connection) else {
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        };
        let uris = match direct.cached_uris(&dest_vmac).await {
            Some(uris) => Some(uris),
            None => {
                // Stopped: the hub socket is gone, and so is discovery.
                let hub = ws_shared
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(crate::port::stopped_for_good)?;
                direct
                    .discover_via_hub(dest_vmac, &hub, conn, self.connect_timeout_ms)
                    .await
            }
        };
        if let Some(uris) = uris.filter(|uris| !uris.is_empty()) {
            match direct
                .try_direct_uris(
                    &uris,
                    dest_vmac,
                    npdu,
                    data_attributes,
                    conn,
                    super::direct_discovery::DirectTiming {
                        connect_timeout_ms: self.connect_timeout_ms,
                        heartbeat_ms: (self.heartbeat_interval_ms, self.heartbeat_timeout_ms),
                    },
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(super::direct_egress::DirectSendError::Unavailable) => {}
                Err(error) => return Err(error.into_error()),
            }
        }
        // Discovery/retirement can race a peer's inbound Connect. Re-evaluate
        // the current route once before selecting Hub for definitely unstarted work.
        if let Some(route) = self.direct_membership.route(&dest_vmac) {
            match route.send_npdu(npdu, data_attributes).await {
                Ok(()) => return Ok(()),
                Err(super::direct_egress::DirectSendError::Unavailable) => {}
                Err(error) => return Err(error.into_error()),
            }
        }
        self.send_via_hub(dest_vmac, npdu, data_attributes).await
    }

    async fn send_via_hub(
        &self,
        dest_vmac: Vmac,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        let ws_shared = self
            .ws_shared
            .as_ref()
            .ok_or_else(|| not_started(&self.stop_gate))?;
        let conn = self
            .connection
            .as_ref()
            .ok_or_else(|| not_started(&self.stop_gate))?;

        // Admission is atomic with socket publication. An admitted send owns
        // its Arc and may complete after retirement; it is not rolled back.
        // A stop takes the socket out of the slot, so a send after it fails
        // here, under the lock every send takes anyway (#2617).
        let (ws, hub_max_bvlc_length, msg) = {
            let ws = ws_shared.lock().await;
            let Some(ws) = ws.as_ref() else {
                return Err(crate::port::stopped_for_good());
            };
            let mut c = conn.lock().await;
            if c.state != ScConnectionState::Connected {
                return Err(Error::state(
                    Lifecycle::NotConnected,
                    "BACnet/SC transport not in Connected state",
                ));
            }
            if npdu.len() > c.hub_max_npdu_length as usize {
                return Err(Error::SendTooLong {
                    length: npdu.len(),
                    limit: usize::from(c.hub_max_npdu_length),
                });
            }
            let hub_max_bvlc_length = c.hub_max_bvlc_length;
            let msg =
                c.build_encapsulated_npdu_with_data_attributes(dest_vmac, npdu, data_attributes)?;
            (Arc::clone(ws), hub_max_bvlc_length, msg)
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if buf.len() > hub_max_bvlc_length as usize {
            return Err(Error::SendTooLong {
                length: buf.len(),
                limit: usize::from(hub_max_bvlc_length),
            });
        }
        ws.send(&buf).await
    }
}

/// What a send before the transport started gets (#1833): it can start,
/// so [`Lifecycle::NotRunning`]; but once a stop came first it can't, so
/// the stopped transport's [`Lifecycle::Closed`], as its start says.
/// Nothing was sent.
#[cold]
#[inline(never)]
fn not_started(gate: &crate::stopping::StopGate) -> Error {
    if gate.is_stopped() {
        return crate::port::stopped_for_good();
    }
    Error::state(Lifecycle::NotRunning, "BACnet/SC transport not started")
}
