//! AcknowledgeAlarm mutation and requester-response construction.

use super::*;

pub(super) async fn response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    accepted: &mut Option<handlers::AcceptedAcknowledgeAlarm>,
) -> Apdu {
    let result = {
        let mut db = db.write().await;
        handlers::handle_acknowledge_alarm(&mut db, &request.service_request)
    };

    match result {
        Ok(result) => {
            *accepted = Some(result);
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            })
        }
        Err(error) => confirmed_response::error_apdu_from_error(
            request.invoke_id,
            request.service_choice,
            &error,
        ),
    }
}
