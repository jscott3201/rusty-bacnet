//! Declared configuration for an unbound, flat application-level B/IP port.
//! The B/IP mode is not configuration here: a registered owner publishes it.

use crate::common;
use bacnet_types::error::Error;

/// Complete configured B/IP snapshot. Fixed-size addresses prevent malformed
/// IPv4 values. Zero address/mask/gateway and one zero DNS address mean unknown
/// or unconfigured; they do not describe an observed interface or route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BipPortConfig {
    /// Caller-configured number; zero means unknown, 65535 is invalid.
    pub network_number: u16,
    /// Port APDU_Length (399), independent of Device Max_APDU_Length_Accepted.
    /// Any Unsigned value >= 50 is permitted by the property clause.
    pub apdu_length: u32,
    /// Configured IPv4 address, in network byte order.
    pub ip_address: [u8; 4],
    /// Zero is allowed in this unbound configuration, independently of Port ID.
    pub udp_port: u16,
    /// Configured mask, with no assumed prefix or interface discovery.
    pub subnet_mask: [u8; 4],
    /// Configured gateway, with no assumed OS route.
    pub default_gateway: [u8; 4],
    /// At least one address; a zero address represents unavailable DNS.
    pub dns_servers: Vec<[u8; 4]>,
}

impl Default for BipPortConfig {
    fn default() -> Self {
        Self {
            network_number: 0,
            // Declared B/IP capacity used by this library's transport profile.
            // No active transport is discovered or verified by this object.
            apdu_length: 1476,
            ip_address: [0; 4],
            udp_port: 47808,
            subnet_mask: [0; 4],
            default_gateway: [0; 4],
            dns_servers: vec![[0; 4]],
        }
    }
}

impl BipPortConfig {
    pub(super) fn validate(&self, instance: u32) -> Result<(), Error> {
        // Local policy for the explicit flat B/IP application profile only.
        if !(1..=255).contains(&instance)
            || self.network_number == u16::MAX
            || self.apdu_length < 50
            || self.dns_servers.is_empty()
        {
            return Err(common::value_out_of_range_error());
        }
        Ok(())
    }
}
