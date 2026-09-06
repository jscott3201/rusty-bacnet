use super::WebSocketPort;
use crate::sc_frame::{validate_control, ControlRecipient, ScMessage};
use tracing::warn;

pub(super) async fn reject_invalid_control<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
) -> bool {
    let Err(nak) = validate_control(msg, wire, ControlRecipient::HubConnector) else {
        return false;
    };
    if let Some(nak) = nak {
        if let Err(e) = ws.send(&nak).await {
            warn!("BACnet/SC control NAK send error: {e}");
        }
    }
    true
}
