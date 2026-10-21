//! `--foreign-bbmd`: run the CLI's own B/IP transport as a foreign device
//! (#938), so its broadcasts reach a remote subnet through that BBMD.
//!
//! The transport registers at start and renews on its own. A one-shot
//! command waits for the first registration to be answered before it runs,
//! and fails when the BBMD refuses it or doesn't answer; the TUI reports
//! the outcome and carries on.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::{
    BipTransport, ForeignDeviceConfig, ForeignRegistrationOutcome, DEFAULT_BACNET_PORT,
};

/// How long a one-shot command waits for the first registration's answer.
/// The transport gives an attempt up after at most three seconds, so this
/// is a backstop for an attempt that never resolves.
const REGISTRATION_WAIT: Duration = Duration::from_secs(5);

/// Parse a `--foreign-bbmd` address: `ip` or `ip:port` (47808 by default).
pub(crate) fn parse_bbmd_arg(value: &str) -> Result<SocketAddrV4, String> {
    let address = match value.parse::<SocketAddrV4>() {
        Ok(address) => address,
        Err(_) => {
            let ip = value
                .parse::<Ipv4Addr>()
                .map_err(|_| format!("'{value}' is not an IPv4 address or ip:port"))?;
            SocketAddrV4::new(ip, DEFAULT_BACNET_PORT)
        }
    };
    if address.port() == 0 || address.ip().is_unspecified() {
        return Err(format!(
            "'{value}' is not a BBMD a device can register with"
        ));
    }
    Ok(address)
}

/// The foreign-device configuration the flags ask for, if any.
pub(crate) fn config(
    bbmd: Option<SocketAddrV4>,
    ttl: u16,
    renewal_secs: Option<u64>,
) -> Option<ForeignDeviceConfig> {
    bbmd.map(|bbmd| ForeignDeviceConfig {
        bbmd_ip: *bbmd.ip(),
        bbmd_port: bbmd.port(),
        ttl,
        renewal_interval: renewal_secs.map(Duration::from_secs),
    })
}

/// Wait until the first registration with `bbmd` is answered: `Ok` once the
/// BBMD accepts it, an error naming the outcome otherwise.
pub(crate) async fn await_registration(
    client: &BACnetClient<BipTransport>,
    bbmd: SocketAddrV4,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + REGISTRATION_WAIT;
    loop {
        let registration = client.foreign_registration().map_err(|e| e.to_string())?;
        match registration.last_outcome {
            Some(ForeignRegistrationOutcome::Accepted) => return Ok(()),
            Some(ForeignRegistrationOutcome::Refused) => {
                let result = registration
                    .last_result
                    .map_or_else(String::new, |result| format!(" ({result})"));
                return Err(format!(
                    "BBMD {bbmd} refused the foreign-device registration{result}"
                ));
            }
            Some(ForeignRegistrationOutcome::LocalError) => {
                return Err(format!(
                    "could not send the foreign-device registration to BBMD {bbmd}"
                ))
            }
            Some(ForeignRegistrationOutcome::TimedOut | ForeignRegistrationOutcome::Cancelled) => {
                return Err(format!(
                    "BBMD {bbmd} did not answer the foreign-device registration"
                ))
            }
            Some(ForeignRegistrationOutcome::Pending) | None => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "the foreign-device registration with BBMD {bbmd} had no outcome after {} s",
                REGISTRATION_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
