use wasm_bindgen::prelude::*;

pub mod codec;
pub mod data_attributes;
pub mod sc_connection;
pub mod sc_frame;
pub mod sc_websocket;
pub mod types;

#[cfg(test)]
mod sc_data_attribute_tests;

// Only compile browser-dependent modules on wasm32 target
#[cfg(target_arch = "wasm32")]
pub mod client;
#[cfg(target_arch = "wasm32")]
pub mod ws_transport;

/// Returns the crate version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
