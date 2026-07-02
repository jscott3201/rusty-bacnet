//! BACnet/SC thin client for JavaScript/TypeScript consumers.
//!
//! This is the main entry point for JS code. It wraps the SC connection state
//! machine, browser WebSocket, and service codecs into a high-level async API.

mod lifecycle;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bytes::BytesMut;
use js_sys::{Array, Function, Reflect, Uint8Array};
use serde::Serialize;
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};

use crate::codec;
use crate::data_attributes::{
    self, DataAttribute, MAX_SC_DATA_ATTRIBUTES, MAX_SC_DATA_ATTRIBUTE_PAYLOAD,
};
use crate::sc_connection::{ScConnection, ScConnectionState};
use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScFunction, Vmac, BROADCAST_VMAC,
};
use crate::ws_transport::BrowserWebSocket;

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
    on_npdu: Rc<RefCell<Option<Function>>>,
    heartbeat_interval_id: Rc<RefCell<Option<i32>>>,
    heartbeat_interval_closure: Rc<RefCell<Option<Closure<dyn FnMut()>>>>,
}

#[derive(Serialize)]
struct ReceivedNpduMetadata {
    source_vmac: Vec<u8>,
    data_attributes: Vec<DataAttribute>,
}

#[wasm_bindgen]
impl BACnetScClient {
    /// Create a new BACnet/SC client with a random VMAC and Device UUID.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Self, JsError> {
        let device_uuid = Self::generate_device_uuid()?;
        Self::with_validated_device_uuid(device_uuid)
    }

    /// Create a new BACnet/SC client with a caller-supplied persistent Device UUID.
    #[wasm_bindgen(js_name = withDeviceUuid)]
    pub fn with_device_uuid(device_uuid: &[u8]) -> Result<Self, JsError> {
        let device_uuid = Self::parse_device_uuid(device_uuid)?;
        Self::with_validated_device_uuid(device_uuid)
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
        let response = match ws.recv().await {
            Ok(response) => response,
            Err(e) => {
                self.connection.borrow_mut().abort_connect();
                ws.close();
                return Err(JsError::new(&e));
            }
        };
        let msg = match decode_sc_message(&response) {
            Ok(msg) => msg,
            Err(e) => {
                self.connection.borrow_mut().abort_connect();
                ws.close();
                return Err(JsError::new(&e.to_string()));
            }
        };
        if msg.function == ScFunction::Result {
            let result = match decode_sc_bvlc_result(&msg) {
                Ok(result) => result,
                Err(e) => {
                    self.connection.borrow_mut().abort_connect();
                    ws.close();
                    return Err(JsError::new(&e.to_string()));
                }
            };
            let needs_replacement = self
                .connection
                .borrow()
                .connect_result_requires_random48_vmac(msg.message_id, &result);
            let replacement_vmac = if needs_replacement {
                match Self::generate_random48_vmac() {
                    Ok(vmac) => Some(vmac),
                    Err(e) => {
                        self.connection.borrow_mut().abort_connect();
                        ws.close();
                        return Err(e);
                    }
                }
            } else {
                None
            };
            let duplicate_vmac = self
                .connection
                .borrow_mut()
                .handle_connect_result(msg.message_id, &result, replacement_vmac)
                .map_err(|e| JsError::new(&e.to_string()))?;
            ws.close();
            return Err(JsError::new(if duplicate_vmac {
                "BACnet/SC ConnectRequest rejected with duplicate VMAC; selected new Random-48 VMAC"
            } else {
                "BACnet/SC ConnectRequest rejected by BVLC-Result"
            }));
        }
        // Validate the monotonic clock before accepting the connection state.
        let heartbeat_start_ms = match Self::monotonic_now_ms() {
            Ok(now_ms) => now_ms,
            Err(e) => {
                self.connection.borrow_mut().abort_connect();
                ws.close();
                return Err(e);
            }
        };
        if !self.connection.borrow_mut().handle_connect_accept(&msg) {
            self.connection.borrow_mut().abort_connect();
            ws.close();
            return Err(JsError::new("ConnectAccept not received or invalid"));
        }
        self.connection
            .borrow_mut()
            .start_heartbeat_tracking(heartbeat_start_ms);

        *self.ws.borrow_mut() = Some(ws);

        if let Err(e) = self.start_heartbeat_loop() {
            Self::terminate_connection(
                &self.ws,
                &self.connection,
                &self.pending,
                &self.heartbeat_interval_id,
                &self.heartbeat_interval_closure,
                "BACnet/SC heartbeat timer startup failed",
            );
            return Err(e);
        }
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
        self.send_npdu_to(BROADCAST_VMAC, &npdu_bytes)?;
        Ok(())
    }

    /// Broadcast raw NPDU bytes through the established BACnet/SC hub connection.
    #[wasm_bindgen(js_name = sendNpdu)]
    pub fn send_raw_npdu(&self, npdu_bytes: &[u8]) -> Result<(), JsError> {
        self.send_npdu_with_attributes_to(BROADCAST_VMAC, npdu_bytes, &[])
    }

    /// Send raw NPDU bytes to a destination VMAC through the established BACnet/SC hub connection.
    #[wasm_bindgen(js_name = sendNpduTo)]
    pub fn send_raw_npdu_to(
        &self,
        destination_vmac: &[u8],
        npdu_bytes: &[u8],
    ) -> Result<(), JsError> {
        let destination_vmac = Self::parse_vmac(destination_vmac)?;
        self.send_npdu_with_attributes_to(destination_vmac, npdu_bytes, &[])
    }

    /// Broadcast raw NPDU bytes with BACnet/SC Data Options.
    ///
    /// `dataAttributes` is an array of objects with `option_type`,
    /// `must_understand`, and `data` fields. `optionType` and
    /// `mustUnderstand` aliases are also accepted.
    #[wasm_bindgen(js_name = sendNpduWithDataAttributes)]
    pub fn send_raw_npdu_with_data_attributes(
        &self,
        npdu_bytes: &[u8],
        data_attributes: JsValue,
    ) -> Result<(), JsError> {
        let data_attributes = Self::parse_data_attributes(&data_attributes)?;
        self.send_npdu_with_attributes_to(BROADCAST_VMAC, npdu_bytes, &data_attributes)
    }

    /// Send raw NPDU bytes to a destination VMAC with BACnet/SC Data Options.
    ///
    /// `dataAttributes` is an array of objects with `option_type`,
    /// `must_understand`, and `data` fields. `optionType` and
    /// `mustUnderstand` aliases are also accepted.
    #[wasm_bindgen(js_name = sendNpduToWithDataAttributes)]
    pub fn send_raw_npdu_to_with_data_attributes(
        &self,
        destination_vmac: &[u8],
        npdu_bytes: &[u8],
        data_attributes: JsValue,
    ) -> Result<(), JsError> {
        let destination_vmac = Self::parse_vmac(destination_vmac)?;
        let data_attributes = Self::parse_data_attributes(&data_attributes)?;
        self.send_npdu_with_attributes_to(destination_vmac, npdu_bytes, &data_attributes)
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

    /// Register a callback for every received NPDU and its BACnet/SC data attributes.
    ///
    /// The callback receives `(npduBytes, metadata)`, where `metadata` contains
    /// `source_vmac` and `data_attributes`.
    #[wasm_bindgen(js_name = onNpdu)]
    pub fn on_npdu(&self, callback: Function) {
        *self.on_npdu.borrow_mut() = Some(callback);
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
        Self::terminate_connection(
            &self.ws,
            &self.connection,
            &self.pending,
            &self.heartbeat_interval_id,
            &self.heartbeat_interval_closure,
            "BACnet/SC client disconnected",
        );
        Ok(())
    }

    /// Check if currently connected.
    #[wasm_bindgen(getter, js_name = connected)]
    pub fn is_connected(&self) -> bool {
        self.connection.borrow().state == ScConnectionState::Connected
    }

    /// Return the local Device UUID used in Connect-Request payloads.
    #[wasm_bindgen(getter, js_name = localDeviceUuid)]
    pub fn local_device_uuid(&self) -> Vec<u8> {
        self.connection.borrow().device_uuid.to_vec()
    }
}

// Private methods
impl BACnetScClient {
    fn with_validated_device_uuid(device_uuid: [u8; 16]) -> Result<Self, JsError> {
        let vmac = Self::generate_random48_vmac()?;

        Ok(Self {
            ws: Rc::new(RefCell::new(None)),
            connection: Rc::new(RefCell::new(ScConnection::new_with_device_uuid(
                vmac,
                device_uuid,
            ))),
            pending: Rc::new(RefCell::new(HashMap::new())),
            next_invoke_id: Rc::new(RefCell::new(0)),
            on_iam: Rc::new(RefCell::new(None)),
            on_cov: Rc::new(RefCell::new(None)),
            on_npdu: Rc::new(RefCell::new(None)),
            heartbeat_interval_id: Rc::new(RefCell::new(None)),
            heartbeat_interval_closure: Rc::new(RefCell::new(None)),
        })
    }

    fn generate_random48_vmac() -> Result<Vmac, JsError> {
        let mut vmac = [0u8; 6];
        Self::fill_secure_random_bytes(&mut vmac)?;
        vmac[0] = (vmac[0] & 0xF0) | 0x02;
        Ok(vmac)
    }

    fn generate_device_uuid() -> Result<[u8; 16], JsError> {
        let mut uuid = [0u8; 16];
        Self::fill_secure_random_bytes(&mut uuid)?;
        uuid[6] = (uuid[6] & 0x0F) | 0x40;
        uuid[8] = (uuid[8] & 0x3F) | 0x80;
        Ok(uuid)
    }

    fn fill_secure_random_bytes(bytes: &mut [u8]) -> Result<(), JsError> {
        let crypto = Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))
            .map_err(|_| JsError::new("WebCrypto crypto is required for BACnet/SC randomness"))?;
        if crypto.is_undefined() || crypto.is_null() {
            return Err(JsError::new(
                "WebCrypto crypto is required for BACnet/SC randomness",
            ));
        }

        let get_random_values = Reflect::get(&crypto, &JsValue::from_str("getRandomValues"))
            .map_err(|_| JsError::new("crypto.getRandomValues is required"))?;
        let get_random_values: Function = get_random_values
            .dyn_into()
            .map_err(|_| JsError::new("crypto.getRandomValues is required"))?;
        let array = Uint8Array::new_with_length(bytes.len() as u32);
        Reflect::apply(&get_random_values, &crypto, &Array::of1(&array))
            .map_err(|_| JsError::new("crypto.getRandomValues failed"))?;
        array.copy_to(bytes);
        Ok(())
    }

    fn parse_device_uuid(device_uuid: &[u8]) -> Result<[u8; 16], JsError> {
        if device_uuid.len() != 16 {
            return Err(JsError::new("Device UUID must be exactly 16 bytes"));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(device_uuid);
        if uuid.iter().all(|byte| *byte == 0) {
            return Err(JsError::new("Device UUID must not be all zero"));
        }
        Ok(uuid)
    }

    fn next_invoke_id(&self) -> u8 {
        let mut id = self.next_invoke_id.borrow_mut();
        let current = *id;
        *id = id.wrapping_add(1);
        current
    }

    fn send_npdu(&self, npdu_bytes: &[u8]) -> Result<(), JsError> {
        self.send_npdu_to(BROADCAST_VMAC, npdu_bytes)
    }

    fn send_npdu_to(&self, destination_vmac: Vmac, npdu_bytes: &[u8]) -> Result<(), JsError> {
        self.send_npdu_with_attributes_to(destination_vmac, npdu_bytes, &[])
    }

    fn send_npdu_with_attributes_to(
        &self,
        destination_vmac: Vmac,
        npdu_bytes: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), JsError> {
        let mut conn = self.connection.borrow_mut();
        if conn.state != ScConnectionState::Connected {
            return Err(JsError::new("not connected"));
        }
        if npdu_bytes.len() > conn.hub_max_apdu_length as usize {
            return Err(JsError::new(&format!(
                "BACnet/SC NPDU length {} exceeds peer Max-NPDU-Length {}",
                npdu_bytes.len(),
                conn.hub_max_apdu_length
            )));
        }
        let hub_max_bvlc_length = conn.hub_max_bvlc_length;
        let data_options_len = data_attributes::encoded_data_options_len(data_attributes)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let encoded_len = 4usize + destination_vmac.len() + data_options_len + npdu_bytes.len();
        if encoded_len > hub_max_bvlc_length as usize {
            return Err(JsError::new(&format!(
                "BACnet/SC encoded BVLC length {} exceeds peer Max-BVLC-Length {}",
                encoded_len, hub_max_bvlc_length
            )));
        }
        let msg = conn
            .build_encapsulated_npdu_with_data_attributes(
                destination_vmac,
                npdu_bytes,
                data_attributes,
            )
            .map_err(|e| JsError::new(&e.to_string()))?;
        drop(conn);

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        if buf.len() > hub_max_bvlc_length as usize {
            return Err(JsError::new(&format!(
                "BACnet/SC encoded BVLC length {} exceeds peer Max-BVLC-Length {}",
                buf.len(),
                hub_max_bvlc_length
            )));
        }
        if let Some(ws) = self.ws.borrow().as_ref() {
            ws.send(&buf)
                .map_err(|e| JsError::new(&format!("{:?}", e)))?;
        }
        Ok(())
    }

    fn parse_vmac(destination_vmac: &[u8]) -> Result<Vmac, JsError> {
        if destination_vmac.len() != 6 {
            return Err(JsError::new(&format!(
                "BACnet/SC VMAC must be 6 bytes, got {}",
                destination_vmac.len()
            )));
        }
        let mut vmac = [0u8; 6];
        vmac.copy_from_slice(destination_vmac);
        Ok(vmac)
    }

    fn parse_data_attributes(value: &JsValue) -> Result<Vec<DataAttribute>, JsError> {
        if !Array::is_array(value) {
            return Err(JsError::new("dataAttributes must be an array"));
        }
        let array: Array = value
            .clone()
            .dyn_into()
            .map_err(|_| JsError::new("dataAttributes must be an array"))?;
        let len = array.length();
        if len as usize > MAX_SC_DATA_ATTRIBUTES {
            return Err(JsError::new(&format!(
                "BACnet/SC Data Options exceed {MAX_SC_DATA_ATTRIBUTES} attributes"
            )));
        }

        let mut attributes = Vec::with_capacity(len as usize);
        for index in 0..len {
            let item = array.get(index);
            let option_type = Self::parse_option_type(&Self::required_alias_property(
                &item,
                "option_type",
                "optionType",
            )?)?;
            let must_understand =
                Self::required_alias_property(&item, "must_understand", "mustUnderstand")?
                    .as_bool()
                    .ok_or_else(|| {
                        JsError::new("DataAttribute.must_understand must be a boolean")
                    })?;
            let data = Self::parse_attribute_data(&Self::required_property(&item, "data")?)?;
            attributes.push(DataAttribute {
                option_type,
                must_understand,
                data,
            });
        }
        Ok(attributes)
    }

    fn required_alias_property(
        object: &JsValue,
        primary: &str,
        alias: &str,
    ) -> Result<JsValue, JsError> {
        let primary_value = Self::reflect_get(object, primary)?;
        if !primary_value.is_undefined() {
            return Ok(primary_value);
        }
        let alias_value = Self::reflect_get(object, alias)?;
        if !alias_value.is_undefined() {
            return Ok(alias_value);
        }
        Err(JsError::new(&format!(
            "DataAttribute.{primary} is required"
        )))
    }

    fn required_property(object: &JsValue, property: &str) -> Result<JsValue, JsError> {
        let value = Self::reflect_get(object, property)?;
        if value.is_undefined() {
            return Err(JsError::new(&format!(
                "DataAttribute.{property} is required"
            )));
        }
        Ok(value)
    }

    fn reflect_get(object: &JsValue, property: &str) -> Result<JsValue, JsError> {
        Reflect::get(object, &JsValue::from_str(property))
            .map_err(|_| JsError::new("DataAttribute entries must be objects"))
    }

    fn parse_option_type(value: &JsValue) -> Result<u8, JsError> {
        let raw = value
            .as_f64()
            .ok_or_else(|| JsError::new("DataAttribute.option_type must be a number"))?;
        if !raw.is_finite() || raw.fract() != 0.0 || !(1.0..=31.0).contains(&raw) {
            return Err(JsError::new(&format!(
                "BACnet/SC Data Option type must be 1..31, got {raw}"
            )));
        }
        Ok(raw as u8)
    }

    fn parse_attribute_data(value: &JsValue) -> Result<Vec<u8>, JsError> {
        if let Some(bytes) = value.dyn_ref::<Uint8Array>() {
            let len = bytes.length() as usize;
            if len > MAX_SC_DATA_ATTRIBUTE_PAYLOAD {
                return Err(JsError::new(&format!(
                    "BACnet/SC Data Option payload length {} exceeds 65535",
                    len
                )));
            }
            let mut data = vec![0u8; len];
            bytes.copy_to(&mut data);
            return Ok(data);
        }

        if !Array::is_array(value) {
            return Err(JsError::new(
                "DataAttribute.data must be a Uint8Array or byte array",
            ));
        }
        let array: Array = value
            .clone()
            .dyn_into()
            .map_err(|_| JsError::new("DataAttribute.data must be a byte array"))?;
        let len = array.length() as usize;
        if len > MAX_SC_DATA_ATTRIBUTE_PAYLOAD {
            return Err(JsError::new(&format!(
                "BACnet/SC Data Option payload length {} exceeds 65535",
                len
            )));
        }

        let mut data = Vec::with_capacity(len);
        for index in 0..array.length() {
            let value = array.get(index);
            let raw = value
                .as_f64()
                .ok_or_else(|| JsError::new("DataAttribute.data entries must be numbers"))?;
            if !raw.is_finite() || raw.fract() != 0.0 || !(0.0..=255.0).contains(&raw) {
                return Err(JsError::new(&format!(
                    "DataAttribute.data entries must be bytes, got {raw}"
                )));
            }
            data.push(raw as u8);
        }
        Ok(data)
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
}
