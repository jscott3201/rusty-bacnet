use bytes::BytesMut;

use bacnet_types::error::Error;

use crate::port::DataAttribute;
use crate::sc_frame::{encode_sc_message, Vmac, BROADCAST_VMAC};

use super::{ScConnectionState, ScTransport, WebSocketPort};

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) async fn send_unicast_inner(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        if self.ws_shared.is_none() {
            return Err(crate::direct_response::unavailable());
        }
        if mac.len() != 6 {
            return Err(Error::Encoding(format!(
                "BACnet/SC VMAC must be 6 bytes, got {}",
                mac.len()
            )));
        }
        let mut dest_vmac = [0u8; 6];
        dest_vmac.copy_from_slice(mac);

        if dest_vmac != BROADCAST_VMAC {
            if let Some(route) = self.direct_membership.route(&dest_vmac) {
                match route.send_npdu(npdu, data_attributes).await {
                    Ok(()) => return Ok(()),
                    Err(super::direct_egress::DirectSendError::Unavailable) => {}
                    Err(error) => return Err(error.into_error()),
                }
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
                let hub = ws_shared.lock().await.clone();
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
                    self.connect_timeout_ms,
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
        let ws_shared = self.ws_shared.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;
        let conn = self.connection.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;

        // Admission is atomic with socket publication. An admitted send owns
        // its Arc and may complete after retirement; it is not rolled back.
        let (ws, hub_max_bvlc_length, msg) = {
            let ws = ws_shared.lock().await;
            let mut c = conn.lock().await;
            if c.state != ScConnectionState::Connected {
                return Err(Error::Encoding(
                    "BACnet/SC transport not in Connected state".into(),
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
            (ws.clone(), hub_max_bvlc_length, msg)
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
