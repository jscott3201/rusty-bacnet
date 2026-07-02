use bytes::BytesMut;

use bacnet_types::error::Error;

use crate::port::DataAttribute;
use crate::sc_frame::encode_sc_message;

use super::{ScConnectionState, ScTransport, WebSocketPort};

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
        let ws = self.ws_shared.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;
        let ws = ws.lock().await.clone();
        let conn = self.connection.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;

        let mut dest_vmac = [0u8; 6];
        dest_vmac.copy_from_slice(mac);

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
        drop(c);

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
