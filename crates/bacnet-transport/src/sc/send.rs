use std::sync::Arc;

use bytes::BytesMut;
use tokio::sync::Mutex;

use bacnet_types::error::Error;

use crate::port::DataAttribute;
use crate::sc_frame::{encode_sc_message, Vmac, BROADCAST_VMAC};

use super::{ScConnection, ScConnectionState, ScTransport, WebSocketPort};

impl<W: WebSocketPort> ScTransport<W> {
    pub(super) async fn send_unicast_inner(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        if mac.len() != 6 {
            return Err(Error::Encoding(format!(
                "BACnet/SC VMAC must be 6 bytes, got {}",
                mac.len()
            )));
        }
        let mut dest_vmac = [0u8; 6];
        dest_vmac.copy_from_slice(mac);

        // Default-off preservation: without opt-in state or for broadcast,
        // run the hub path exactly as before with no extra work.
        let direct = self.direct_shared();
        if direct.is_none() || dest_vmac == BROADCAST_VMAC {
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        }
        let direct = direct.expect("direct opt-in checked above");
        let ws_shared = match self.ws_shared.clone() {
            Some(ws) => ws,
            None => return self.send_via_hub(dest_vmac, npdu, data_attributes).await,
        };
        let conn = match self.connection.clone() {
            Some(conn) => conn,
            None => return self.send_via_hub(dest_vmac, npdu, data_attributes).await,
        };
        let connect_timeout_ms = self.connect_timeout_ms;

        // Cache consult: fresh hit dials direct; fresh empty goes hub.
        if let Some(uris) = direct.cached_uris(&dest_vmac).await {
            if uris.is_empty() {
                return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
            }
            if Self::try_direct_then_hub(
                &direct,
                &uris,
                dest_vmac,
                npdu,
                data_attributes,
                &conn,
                connect_timeout_ms,
            )
            .await
            .is_ok()
            {
                return Ok(());
            }
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        }

        // Miss: one on-demand Address-Resolution through the hub, then dial.
        // Any discovery failure falls back to hub delivery without caching.
        let hub_ws = {
            let guard = ws_shared.lock().await;
            (*guard).clone()
        };
        let discovered = direct
            .discover_via_hub(dest_vmac, &hub_ws, &conn, connect_timeout_ms)
            .await;
        let Some(uris) = discovered else {
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        };
        if uris.is_empty() {
            return self.send_via_hub(dest_vmac, npdu, data_attributes).await;
        }
        if Self::try_direct_then_hub(
            &direct,
            &uris,
            dest_vmac,
            npdu,
            data_attributes,
            &conn,
            connect_timeout_ms,
        )
        .await
        .is_ok()
        {
            return Ok(());
        }
        self.send_via_hub(dest_vmac, npdu, data_attributes).await
    }

    async fn try_direct_then_hub(
        direct: &Arc<super::direct_discovery::DirectShared<W>>,
        uris: &[String],
        dest_vmac: Vmac,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
        conn: &Arc<Mutex<ScConnection>>,
        connect_timeout_ms: u64,
    ) -> Result<(), ()> {
        // Hub limits bound the direct frame until a direct handshake learns
        // peer limits; read them without holding the lock across dial/send.
        let (hub_max_bvlc_length, hub_max_apdu_length) = {
            let c = conn.lock().await;
            (c.hub_max_bvlc_length, c.hub_max_apdu_length)
        };
        direct
            .try_direct_uris(
                uris,
                dest_vmac,
                npdu,
                data_attributes,
                conn,
                hub_max_bvlc_length,
                hub_max_apdu_length,
                connect_timeout_ms,
            )
            .await
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
            if npdu.len() > c.hub_max_apdu_length as usize {
                return Err(Error::Encoding(format!(
                    "BACnet/SC NPDU length {} exceeds peer Max-NPDU-Length {}",
                    npdu.len(),
                    c.hub_max_apdu_length
                )));
            }
            let hub_max_bvlc_length = c.hub_max_bvlc_length;
            let msg =
                c.build_encapsulated_npdu_with_data_attributes(dest_vmac, npdu, data_attributes)?;
            (ws.clone(), hub_max_bvlc_length, msg)
        };

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if buf.len() > hub_max_bvlc_length as usize {
            return Err(Error::Encoding(format!(
                "BACnet/SC encoded BVLC length {} exceeds peer Max-BVLC-Length {}",
                buf.len(),
                hub_max_bvlc_length
            )));
        }
        ws.send(&buf).await
    }
}
