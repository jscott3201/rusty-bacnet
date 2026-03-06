use std::fmt;

use wasm_bindgen::prelude::*;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

/// JS-facing wrapper around `ObjectIdentifier`.
#[wasm_bindgen]
pub struct JsObjectIdentifier(ObjectIdentifier);

impl fmt::Display for JsObjectIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[wasm_bindgen]
impl JsObjectIdentifier {
    #[wasm_bindgen(constructor)]
    pub fn new(object_type: u16, instance: u32) -> Result<JsObjectIdentifier, JsError> {
        ObjectIdentifier::new(ObjectType::from_raw(object_type as u32), instance)
            .map(JsObjectIdentifier)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(getter)]
    pub fn object_type(&self) -> u16 {
        self.0.object_type().to_raw() as u16
    }

    #[wasm_bindgen(getter)]
    pub fn instance_number(&self) -> u32 {
        self.0.instance_number()
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn display(&self) -> String {
        self.to_string()
    }
}

/// Well-known BACnet object type constants for JS consumers.
#[wasm_bindgen]
pub struct ObjectTypes;

#[wasm_bindgen]
impl ObjectTypes {
    pub fn analog_input() -> u16 {
        0
    }
    pub fn analog_output() -> u16 {
        1
    }
    pub fn analog_value() -> u16 {
        2
    }
    pub fn binary_input() -> u16 {
        3
    }
    pub fn binary_output() -> u16 {
        4
    }
    pub fn binary_value() -> u16 {
        5
    }
    pub fn calendar() -> u16 {
        6
    }
    pub fn device() -> u16 {
        8
    }
    pub fn multi_state_input() -> u16 {
        13
    }
    pub fn multi_state_output() -> u16 {
        14
    }
    pub fn notification_class() -> u16 {
        15
    }
    pub fn schedule() -> u16 {
        17
    }
    pub fn multi_state_value() -> u16 {
        19
    }
    pub fn trend_log() -> u16 {
        20
    }
}

/// Well-known BACnet property identifier constants for JS consumers.
#[wasm_bindgen]
pub struct PropertyIds;

#[wasm_bindgen]
impl PropertyIds {
    pub fn present_value() -> u32 {
        85
    }
    pub fn object_name() -> u32 {
        77
    }
    pub fn description() -> u32 {
        28
    }
    pub fn status_flags() -> u32 {
        111
    }
    pub fn out_of_service() -> u32 {
        81
    }
    pub fn units() -> u32 {
        117
    }
    pub fn object_list() -> u32 {
        76
    }
    pub fn object_type() -> u32 {
        79
    }
    pub fn object_identifier() -> u32 {
        75
    }
    pub fn priority_array() -> u32 {
        87
    }
    pub fn relinquish_default() -> u32 {
        104
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_identifier_round_trip() {
        let oid = JsObjectIdentifier::new(0, 1).unwrap();
        assert_eq!(oid.object_type(), 0);
        assert_eq!(oid.instance_number(), 1);
    }

    #[test]
    fn object_identifier_max_instance() {
        let oid = JsObjectIdentifier::new(0, 0x3F_FFFF).unwrap();
        assert_eq!(oid.instance_number(), 0x3F_FFFF);
    }

    #[test]
    fn object_identifier_over_max_instance() {
        // Test underlying ObjectIdentifier validation directly (JsError panics on non-wasm)
        assert!(ObjectIdentifier::new(ObjectType::from_raw(0), 0x40_0000).is_err());
    }

    #[test]
    fn object_types_analog_input() {
        assert_eq!(ObjectTypes::analog_input(), 0);
    }

    #[test]
    fn property_ids_present_value() {
        assert_eq!(PropertyIds::present_value(), 85);
    }
}
