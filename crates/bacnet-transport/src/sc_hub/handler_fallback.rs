//! Fallback reply for an unexpected hub function.
use super::*;

pub(super) async fn send(write: &Arc<Mutex<WsSink>>, message_id: u16, other: ScFunction) {
    let nak = build_bvlc_result_nak(
        message_id,
        other,
        ErrorClass::COMMUNICATION,
        unexpected_bvlc_function_error_code(other),
    );
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &nak);
    let mut w = write.lock().await;
    let _ = w.send(Message::Binary(buf.to_vec().into())).await;
}
