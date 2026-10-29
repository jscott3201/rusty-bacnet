use super::WebSocketPort;
use super::rejection::{RejectionBudget, RejectionExpired};
use crate::sc_frame::{ControlRecipient, ScMessage, validate_control};
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
    if let Some(nak) = nak
        && let Err(e) = budget.send(ws, &nak).await?
    {
        warn!("BACnet/SC control NAK send error: {e}");
    }
    Ok(true)
}
