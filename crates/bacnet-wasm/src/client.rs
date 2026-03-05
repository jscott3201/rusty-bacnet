//! BACnet/SC thin client for JavaScript/TypeScript consumers.
//!
//! This is the main entry point for JS code. It wraps the SC connection state
//! machine, browser WebSocket, and service codecs into a high-level async API.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bytes::BytesMut;
use js_sys::Function;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::codec;
use crate::sc_connection::{ScConnection, ScConnectionState};
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction};
use crate::ws_transport::BrowserWebSocket;
use bacnet_encoding::apdu;
use bacnet_encoding::npdu;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};

/// BACnet/SC thin client for browser environments.
///
/// ```js
/// const client = new BACnetScClient();
/// await client.connect("wss://hub.example.com:1234");
/// const value = await client.readProperty(0, 1, 85); // AI:1, PresentValue
/// ```
#[wasm_bindgen]
pub struct BACnetScClient {
    ws: Rc<RefCell<Option<BrowserWebSocket>>>,
    connection: Rc<RefCell<ScConnection>>,
    /// Pending confirmed requests: invoke_id → (resolve, reject)
    pending: Rc<RefCell<HashMap<u8, (Function, Function)>>>,
    next_invoke_id: Rc<RefCell<u8>>,
    on_iam: Rc<RefCell<Option<Function>>>,
    on_cov: Rc<RefCell<Option<Function>>>,
}

#[wasm_bindgen]
impl BACnetScClient {
    /// Create a new BACnet/SC client with a random VMAC.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        // Generate random 6-byte VMAC
        let mut vmac = [0u8; 6];
        let crypto = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))
            .ok()
            .and_then(|c| js_sys::Reflect::get(&c, &JsValue::from_str("getRandomValues")).ok());
        if crypto.is_some() {
            let array = js_sys::Uint8Array::new_with_length(6);
            let _ = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("crypto")).and_then(
                |c| {
                    js_sys::Reflect::apply(
                        &js_sys::Function::from(
                            js_sys::Reflect::get(&c, &JsValue::from_str("getRandomValues"))
                                .unwrap(),
                        ),
                        &c,
                        &js_sys::Array::of1(&array),
                    )
                },
            );
            array.copy_to(&mut vmac);
        } else {
            // Fallback: use js_sys::Math::random
            for byte in &mut vmac {
                *byte = (js_sys::Math::random() * 256.0) as u8;
            }
        }

        Self {
            ws: Rc::new(RefCell::new(None)),
            connection: Rc::new(RefCell::new(ScConnection::new(vmac))),
            pending: Rc::new(RefCell::new(HashMap::new())),
            next_invoke_id: Rc::new(RefCell::new(0)),
            on_iam: Rc::new(RefCell::new(None)),
            on_cov: Rc::new(RefCell::new(None)),
        }
    }

    /// Connect to a BACnet/SC hub via WebSocket.
    pub async fn connect(&self, url: &str) -> Result<(), JsError> {
        let ws = BrowserWebSocket::connect(url)
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        // Send ConnectRequest
        let req = self.connection.borrow_mut().build_connect_request();
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &req);
        ws.send(&buf)
            .map_err(|e| JsError::new(&format!("{:?}", e)))?;

        // Wait for ConnectAccept
        let response = ws.recv().await.map_err(|e| JsError::new(&e))?;
        let msg = decode_sc_message(&response).map_err(|e| JsError::new(&e.to_string()))?;
        if !self.connection.borrow_mut().handle_connect_accept(&msg) {
            return Err(JsError::new("ConnectAccept not received or invalid"));
        }

        *self.ws.borrow_mut() = Some(ws);

        // Start receive loop
        self.start_recv_loop();

        Ok(())
    }

    /// Read a property from a remote BACnet device.
    #[wasm_bindgen(js_name = readProperty)]
    pub async fn read_property(
        &self,
        object_type: u32,
        instance: u32,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Result<JsValue, JsError> {
        let invoke_id = self.next_invoke_id();
        let npdu_bytes = codec::encode_read_property(
            invoke_id,
            object_type,
            instance,
            property_id,
            array_index,
        )?;

        let response = self.send_confirmed(&npdu_bytes, invoke_id).await?;
        Ok(response)
    }

    /// Write a property on a remote BACnet device.
    #[wasm_bindgen(js_name = writeProperty)]
    pub async fn write_property(
        &self,
        object_type: u32,
        instance: u32,
        property_id: u32,
        value_bytes: &[u8],
        priority: Option<u8>,
    ) -> Result<(), JsError> {
        let invoke_id = self.next_invoke_id();
        let npdu_bytes = codec::encode_write_property(
            invoke_id,
            object_type,
            instance,
            property_id,
            value_bytes,
            priority,
        )?;

        self.send_confirmed(&npdu_bytes, invoke_id).await?;
        Ok(())
    }

    /// Send a Who-Is broadcast request.
    #[wasm_bindgen(js_name = whoIs)]
    pub fn who_is(&self, low: Option<u32>, high: Option<u32>) -> Result<(), JsError> {
        let npdu_bytes = codec::encode_who_is(low, high)?;
        self.send_npdu(&npdu_bytes)?;
        Ok(())
    }

    /// Subscribe to COV notifications for an object.
    #[wasm_bindgen(js_name = subscribeCov)]
    pub async fn subscribe_cov(
        &self,
        process_id: u32,
        object_type: u32,
        instance: u32,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), JsError> {
        let invoke_id = self.next_invoke_id();
        let npdu_bytes = codec::encode_subscribe_cov(
            invoke_id,
            process_id,
            object_type,
            instance,
            confirmed,
            lifetime,
        )?;

        self.send_confirmed(&npdu_bytes, invoke_id).await?;
        Ok(())
    }

    /// Register a callback for I-Am responses.
    #[wasm_bindgen(js_name = onIAm)]
    pub fn on_iam(&self, callback: Function) {
        *self.on_iam.borrow_mut() = Some(callback);
    }

    /// Register a callback for COV notifications.
    #[wasm_bindgen(js_name = onCovNotification)]
    pub fn on_cov_notification(&self, callback: Function) {
        *self.on_cov.borrow_mut() = Some(callback);
    }

    /// Disconnect from the hub.
    pub async fn disconnect(&self) -> Result<(), JsError> {
        if let Ok(msg) = self.connection.borrow_mut().build_disconnect_request() {
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &msg);
            if let Some(ws) = self.ws.borrow().as_ref() {
                let _ = ws.send(&buf);
            }
        }
        if let Some(ws) = self.ws.borrow().as_ref() {
            ws.close();
        }
        self.connection.borrow_mut().state = ScConnectionState::Disconnected;
        Ok(())
    }

    /// Check if currently connected.
    #[wasm_bindgen(getter, js_name = connected)]
    pub fn is_connected(&self) -> bool {
        self.connection.borrow().state == ScConnectionState::Connected
    }
}

// Private methods
impl BACnetScClient {
    fn next_invoke_id(&self) -> u8 {
        let mut id = self.next_invoke_id.borrow_mut();
        let current = *id;
        *id = id.wrapping_add(1);
        current
    }

    fn send_npdu(&self, npdu_bytes: &[u8]) -> Result<(), JsError> {
        let conn = self.connection.borrow_mut();
        if conn.state != ScConnectionState::Connected {
            return Err(JsError::new("not connected"));
        }
        let hub_vmac = conn.hub_vmac.unwrap_or([0xFF; 6]);
        drop(conn);

        let msg = self
            .connection
            .borrow_mut()
            .build_encapsulated_npdu(hub_vmac, npdu_bytes);
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if let Some(ws) = self.ws.borrow().as_ref() {
            ws.send(&buf)
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        }
        Ok(())
    }

    async fn send_confirmed(&self, npdu_bytes: &[u8], invoke_id: u8) -> Result<JsValue, JsError> {
        self.send_npdu(npdu_bytes)?;

        // Create a Promise that resolves when the response arrives
        let pending = self.pending.clone();
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            pending.borrow_mut().insert(invoke_id, (resolve, reject));
        });
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|e| JsError::new(&format!("{:?}", e)))
    }

    fn start_recv_loop(&self) {
        let ws = self.ws.clone();
        let connection = self.connection.clone();
        let pending = self.pending.clone();
        let on_iam = self.on_iam.clone();
        let on_cov = self.on_cov.clone();

        spawn_local(async move {
            loop {
                let data = {
                    let ws_ref = ws.borrow();
                    let Some(ws) = ws_ref.as_ref() else {
                        break;
                    };
                    match ws.recv().await {
                        Ok(data) => data,
                        Err(_) => break,
                    }
                };

                // Decode SC frame
                let Ok(sc_msg) = decode_sc_message(&data) else {
                    continue;
                };

                // Handle SC message
                {
                    let npdu_data = connection.borrow_mut().handle_received(&sc_msg);
                    if let Some((npdu_bytes, _source)) = npdu_data {
                        Self::process_npdu(&npdu_bytes, &pending, &on_iam, &on_cov);
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
                }

                // Handle heartbeat
                if sc_msg.function == ScFunction::HeartbeatRequest {
                    let ack = crate::sc_frame::ScMessage {
                        function: ScFunction::HeartbeatAck,
                        message_id: sc_msg.message_id,
                        originating_vmac: sc_msg.destination_vmac,
                        destination_vmac: sc_msg.originating_vmac,
                        dest_options: Vec::new(),
                        data_options: Vec::new(),
                        payload: bytes::Bytes::new(),
                    };
                    let mut buf = BytesMut::new();
                    encode_sc_message(&mut buf, &ack);
                    if let Some(ws) = ws.borrow().as_ref() {
                        let _ = ws.send(&buf);
                    }
                }
            }
        });
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
}
