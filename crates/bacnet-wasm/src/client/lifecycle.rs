use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bytes::BytesMut;
use js_sys::{Function, Uint8Array};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;

use super::{BACnetScClient, ReceivedNpduMetadata};
use crate::codec;
use crate::data_attributes;
use crate::sc_connection::ReceivedScNpdu;
use crate::sc_connection::{
    ScConnection, ScConnectionState, ScHeartbeatAction, DEFAULT_HEARTBEAT_INTERVAL_MS,
    DEFAULT_HEARTBEAT_TIMEOUT_MS,
};
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction};
use crate::ws_transport::BrowserWebSocket;
use bacnet_encoding::apdu;
use bacnet_encoding::npdu;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};

impl BACnetScClient {
    pub(super) fn start_recv_loop(&self) {
        let ws = self.ws.clone();
        let connection = self.connection.clone();
        let pending = self.pending.clone();
        let on_iam = self.on_iam.clone();
        let on_cov = self.on_cov.clone();
        let on_npdu = self.on_npdu.clone();
        let heartbeat_interval_id = self.heartbeat_interval_id.clone();
        let heartbeat_interval_closure = self.heartbeat_interval_closure.clone();

        spawn_local(async move {
            loop {
                let ws_handle = {
                    let ws_ref = ws.borrow();
                    let Some(ws) = ws_ref.as_ref() else {
                        break;
                    };
                    ws.clone()
                };
                let recv_result = ws_handle.recv().await;
                let data = match recv_result {
                    Ok(data) => data,
                    Err(_) => {
                        Self::terminate_connection(
                            &ws,
                            &connection,
                            &pending,
                            &heartbeat_interval_id,
                            &heartbeat_interval_closure,
                            "BACnet/SC WebSocket receive loop ended",
                        );
                        break;
                    }
                };

                // Decode SC frame
                let max_bvlc = connection.borrow().max_bvlc_length as usize;
                if data.len() > max_bvlc {
                    continue;
                }
                let sc_msg = match decode_sc_message(&data) {
                    Ok(sc_msg) => sc_msg,
                    Err(_) => {
                        if let Some(result) =
                            data_attributes::malformed_secure_path_result_from_frame(&data)
                        {
                            if let Some(nak) = result {
                                let mut buf = BytesMut::new();
                                encode_sc_message(&mut buf, &nak);
                                if let Some(ws) = ws.borrow().as_ref() {
                                    let _ = ws.send(&buf);
                                }
                            }
                        }
                        continue;
                    }
                };

                let now_ms = match Self::monotonic_now_ms() {
                    Ok(now_ms) => now_ms,
                    Err(_) => {
                        Self::terminate_connection(
                            &ws,
                            &connection,
                            &pending,
                            &heartbeat_interval_id,
                            &heartbeat_interval_closure,
                            "BACnet/SC monotonic clock unavailable",
                        );
                        break;
                    }
                };
                if sc_msg.function == ScFunction::HeartbeatAck {
                    connection
                        .borrow_mut()
                        .handle_heartbeat_ack(&sc_msg, now_ms);
                    continue;
                }
                connection.borrow_mut().record_heartbeat_activity(now_ms);

                let rejection = {
                    connection
                        .borrow()
                        .unsupported_must_understand_result(&sc_msg)
                };
                if let Some(result) = rejection {
                    if let Some(nak) = result {
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &nak);
                        if let Some(ws) = ws.borrow().as_ref() {
                            let _ = ws.send(&buf);
                        }
                    }
                    continue;
                }

                // Handle SC message
                {
                    let npdu_data = connection.borrow_mut().handle_received(&sc_msg);
                    if let Some(received) = npdu_data {
                        Self::emit_npdu(&received, &on_npdu);
                        Self::process_npdu(&received.npdu, &pending, &on_iam, &on_cov);
                    }
                    // Send disconnect ACK if pending
                    let ack = connection.borrow_mut().disconnect_ack_to_send.take();
                    if let Some(ack) = ack {
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &ack);
                        if let Some(ws) = ws.borrow().as_ref() {
                            let _ = ws.send(&buf);
                        }
                    }
                    if connection.borrow().state == ScConnectionState::Disconnected {
                        Self::terminate_connection(
                            &ws,
                            &connection,
                            &pending,
                            &heartbeat_interval_id,
                            &heartbeat_interval_closure,
                            "BACnet/SC peer disconnected or sent fatal Result",
                        );
                        break;
                    }
                }

                // Handle heartbeat
                if sc_msg.function == ScFunction::HeartbeatRequest {
                    let ack = connection.borrow().build_heartbeat_ack(sc_msg.message_id);
                    let mut buf = BytesMut::new();
                    encode_sc_message(&mut buf, &ack);
                    if let Some(ws) = ws.borrow().as_ref() {
                        let _ = ws.send(&buf);
                    }
                }
            }
        });
    }

    pub(super) fn start_heartbeat_loop(&self) -> Result<(), JsError> {
        self.stop_heartbeat_loop();

        let window = web_sys::window().ok_or_else(|| JsError::new("browser Window is required"))?;
        let performance = window
            .performance()
            .ok_or_else(|| JsError::new("browser Performance clock is required"))?;
        let ws = self.ws.clone();
        let connection = self.connection.clone();
        let pending = self.pending.clone();
        let heartbeat_interval_id = self.heartbeat_interval_id.clone();
        let heartbeat_interval_closure = self.heartbeat_interval_closure.clone();

        let closure = Closure::<dyn FnMut()>::new(move || {
            let action = connection.borrow_mut().next_heartbeat_action(
                performance.now() as u64,
                DEFAULT_HEARTBEAT_INTERVAL_MS,
                DEFAULT_HEARTBEAT_TIMEOUT_MS,
            );

            if let ScHeartbeatAction::Send(message) = action {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &message);
                let sent = ws
                    .borrow()
                    .as_ref()
                    .map(|ws| ws.send(&buf).is_ok())
                    .unwrap_or(false);
                if !sent {
                    connection.borrow_mut().mark_disconnected();
                }
            }

            if connection.borrow().state == ScConnectionState::Disconnected {
                Self::terminate_connection_from_heartbeat_callback(
                    &ws,
                    &connection,
                    &pending,
                    &heartbeat_interval_id,
                    &heartbeat_interval_closure,
                    "BACnet/SC heartbeat failed",
                );
            }
        });

        let interval_id = window
            .set_interval_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                DEFAULT_HEARTBEAT_INTERVAL_MS as i32,
            )
            .map_err(|_| JsError::new("failed to start BACnet/SC heartbeat timer"))?;

        *self.heartbeat_interval_id.borrow_mut() = Some(interval_id);
        *self.heartbeat_interval_closure.borrow_mut() = Some(closure);
        Ok(())
    }

    fn stop_heartbeat_loop(&self) {
        Self::clear_heartbeat_timer(
            &self.heartbeat_interval_id,
            &self.heartbeat_interval_closure,
        );
    }

    pub(super) fn monotonic_now_ms() -> Result<u64, JsError> {
        let window = web_sys::window().ok_or_else(|| JsError::new("browser Window is required"))?;
        let performance = window
            .performance()
            .ok_or_else(|| JsError::new("browser Performance clock is required"))?;
        Ok(performance.now() as u64)
    }

    pub(super) fn terminate_connection(
        ws: &Rc<RefCell<Option<BrowserWebSocket>>>,
        connection: &Rc<RefCell<ScConnection>>,
        pending: &Rc<RefCell<HashMap<u8, (Function, Function)>>>,
        heartbeat_interval_id: &Rc<RefCell<Option<i32>>>,
        heartbeat_interval_closure: &Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
        reason: &str,
    ) {
        connection.borrow_mut().mark_disconnected();
        if let Some(ws) = ws.borrow_mut().take() {
            ws.close();
        }
        Self::clear_heartbeat_timer(heartbeat_interval_id, heartbeat_interval_closure);
        Self::reject_pending(pending, reason);
    }

    fn terminate_connection_from_heartbeat_callback(
        ws: &Rc<RefCell<Option<BrowserWebSocket>>>,
        connection: &Rc<RefCell<ScConnection>>,
        pending: &Rc<RefCell<HashMap<u8, (Function, Function)>>>,
        heartbeat_interval_id: &Rc<RefCell<Option<i32>>>,
        heartbeat_interval_closure: &Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
        reason: &str,
    ) {
        connection.borrow_mut().mark_disconnected();
        if let Some(ws) = ws.borrow_mut().take() {
            ws.close();
        }
        Self::clear_heartbeat_interval_id(heartbeat_interval_id);
        Self::reject_pending(pending, reason);
        // This path runs inside the interval callback, so drop the retained
        // Closure after the callback returns.
        Self::defer_heartbeat_closure_drop(heartbeat_interval_closure.clone());
    }

    fn defer_heartbeat_closure_drop(
        heartbeat_interval_closure: Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
    ) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let cleanup = Closure::<dyn FnMut()>::once(move || {
            heartbeat_interval_closure.borrow_mut().take();
        });
        if window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                cleanup.as_ref().unchecked_ref(),
                0,
            )
            .is_ok()
        {
            cleanup.forget();
        }
    }

    fn reject_pending(pending: &Rc<RefCell<HashMap<u8, (Function, Function)>>>, reason: &str) {
        let error: JsValue = js_sys::Error::new(reason).into();
        for (_invoke_id, (_resolve, reject)) in pending.borrow_mut().drain() {
            let _ = reject.call1(&JsValue::NULL, &error);
        }
    }

    fn clear_heartbeat_timer(
        interval_id: &Rc<RefCell<Option<i32>>>,
        interval_closure: &Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
    ) {
        Self::clear_heartbeat_interval_id(interval_id);
        interval_closure.borrow_mut().take();
    }

    fn clear_heartbeat_interval_id(interval_id: &Rc<RefCell<Option<i32>>>) {
        if let Some(id) = interval_id.borrow_mut().take() {
            if let Some(window) = web_sys::window() {
                window.clear_interval_with_handle(id);
            }
        }
    }

    fn process_npdu(
        npdu_bytes: &[u8],
        pending: &Rc<RefCell<HashMap<u8, (Function, Function)>>>,
        on_iam: &Rc<RefCell<Option<Function>>>,
        on_cov: &Rc<RefCell<Option<Function>>>,
    ) {
        // Decode NPDU to get APDU
        let Ok(npdu) = npdu::decode_npdu(bytes::Bytes::copy_from_slice(npdu_bytes)) else {
            return;
        };
        let Ok(apdu_result) = apdu::decode_apdu(npdu.payload.clone()) else {
            return;
        };

        match apdu_result {
            apdu::Apdu::ComplexAck(ack) => {
                if let Some((resolve, _reject)) = pending.borrow_mut().remove(&ack.invoke_id) {
                    // Decode based on service choice
                    let result = if ack.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
                        codec::decode_read_property_ack(&ack.service_ack).unwrap_or(JsValue::NULL)
                    } else {
                        JsValue::TRUE
                    };
                    let _ = resolve.call1(&JsValue::NULL, &result);
                }
            }
            apdu::Apdu::SimpleAck(ack) => {
                if let Some((resolve, _reject)) = pending.borrow_mut().remove(&ack.invoke_id) {
                    let _ = resolve.call1(&JsValue::NULL, &JsValue::TRUE);
                }
            }
            apdu::Apdu::Error(err) => {
                if let Some((_resolve, reject)) = pending.borrow_mut().remove(&err.invoke_id) {
                    let msg = format!(
                        "BACnet error: class={} code={}",
                        err.error_class.to_raw(),
                        err.error_code.to_raw()
                    );
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&msg));
                }
            }
            apdu::Apdu::Reject(rej) => {
                if let Some((_resolve, reject)) = pending.borrow_mut().remove(&rej.invoke_id) {
                    let msg = format!("BACnet reject: reason={}", rej.reject_reason.to_raw());
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&msg));
                }
            }
            apdu::Apdu::Abort(abt) => {
                if let Some((_resolve, reject)) = pending.borrow_mut().remove(&abt.invoke_id) {
                    let msg = format!("BACnet abort: reason={}", abt.abort_reason.to_raw());
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&msg));
                }
            }
            apdu::Apdu::UnconfirmedRequest(req) => {
                if req.service_choice == UnconfirmedServiceChoice::I_AM {
                    if let Some(cb) = on_iam.borrow().as_ref() {
                        let _ = cb.call1(
                            &JsValue::NULL,
                            &js_sys::Uint8Array::from(req.service_request.as_ref()),
                        );
                    }
                } else if req.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                {
                    if let Some(cb) = on_cov.borrow().as_ref() {
                        let _ = cb.call1(
                            &JsValue::NULL,
                            &js_sys::Uint8Array::from(req.service_request.as_ref()),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn emit_npdu(received: &ReceivedScNpdu, on_npdu: &Rc<RefCell<Option<Function>>>) {
        let Some(callback) = on_npdu.borrow().as_ref().cloned() else {
            return;
        };

        let metadata = ReceivedNpduMetadata {
            source_vmac: received.source_vmac.to_vec(),
            data_attributes: received.data_attributes.clone(),
        };
        let metadata = serde_wasm_bindgen::to_value(&metadata).unwrap_or(JsValue::NULL);
        let _ = callback.call2(
            &JsValue::NULL,
            &Uint8Array::from(received.npdu.as_ref()),
            &metadata,
        );
    }
}
