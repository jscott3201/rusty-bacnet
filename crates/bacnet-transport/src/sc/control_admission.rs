use super::rejection::{RejectionBudget, RejectionExpired};
use super::WebSocketPort;
use crate::sc_frame::{validate_control, ControlRecipient, ScMessage};
use tracing::warn;

pub(super) async fn reject_invalid_control<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    let Err(nak) = validate_control(msg, wire, ControlRecipient::HubConnector) else {
        return Ok(false);
    };
    if let Some(nak) = nak {
        if let Err(e) = budget.send(ws, &nak).await? {
            warn!("BACnet/SC control NAK send error: {e}");
        }
    }
    Ok(true)
}
