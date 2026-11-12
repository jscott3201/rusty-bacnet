//! AcknowledgeAlarm mutation and requester-response construction.

use super::*;
use bacnet_device_core::event::{AcceptedAcknowledgeAlarm, handle_acknowledge_alarm};

pub(super) async fn response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    accepted: &mut Option<AcceptedAcknowledgeAlarm>,
) -> Apdu {
    let result = {
        let mut db = db.write().await;
        handle_acknowledge_alarm(&mut db, &request.service_request)
    };

    match result {
        Ok(result) => {
            *accepted = Some(result);
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            })
        }
        Err(error) => {
            bacnet_device_core::reply::error_apdu(request.invoke_id, request.service_choice, &error)
        }
    }
}
