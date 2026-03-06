mod client;
mod errors;
mod server;
mod transport;
mod types;

pub use client::{BacnetClient, CovNotificationStream};
pub use errors::BacnetError;
pub use server::BacnetServer;
pub use types::*;

uniffi::setup_scaffolding!();

/// Returns the crate version.
#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!version().is_empty());
    }

    #[test]
    fn test_version_format() {
        let v = version();
        assert!(v.contains('.'), "version should contain dots: {v}");
    }
}
