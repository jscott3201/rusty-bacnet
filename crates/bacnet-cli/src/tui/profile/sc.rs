//! BACnet/SC profile settings. A profile names its TLS files by path only:
//! certificates and keys stay where the site keeps them, and no PEM text,
//! key bytes or password is ever written to a profile.

use crate::transport::{
    DEFAULT_SC_CONNECT_TIMEOUT_MS, check_sc_connect_timeout, parse_sc_device_uuid_arg,
    parse_sc_vmac_arg, same_hub_url,
};
use bacnet_transport::sc::ScReconnectConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::PathBuf;

/// SC network settings. `vmac` and `device_uuid` are filled in when the
/// profile is first saved (or first connected, for a hand-written file) and
/// kept from then on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sc {
    /// The primary hub, a `wss://` URI.
    pub(crate) hub_url: String,
    /// The failover hub, used after reconnects to the primary run out. It
    /// needs `reconnect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failover_hub_url: Option<String>,
    /// Trusted site CA certificates (PEM file).
    pub(crate) ca: PathBuf,
    /// This node's operational certificate (PEM file).
    pub(crate) cert: PathBuf,
    /// This node's private key (PEM file). Only the path is stored.
    pub(crate) key: PathBuf,
    /// The node's VMAC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) vmac: Option<Vmac>,
    /// The device UUID, generated once and then stable (Annex AB.1.5.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) device_uuid: Option<DeviceUuid>,
    /// APDU timeout.
    #[serde(default = "default_timeout_ms")]
    pub(crate) timeout_ms: u64,
    /// Minimum spacing between confirmed requests (0: none).
    #[serde(default)]
    pub(crate) min_interval_ms: u64,
    /// The bound on each hub dial and Connect-Request handshake.
    #[serde(default = "default_connect_timeout_ms")]
    pub(crate) connect_timeout_ms: u64,
    /// Redial a lost hub with backoff (#1831). On by default, so a long
    /// session survives a hub restart; `false` leaves a lost connection
    /// down, and is refused with a failover hub.
    #[serde(default = "default_reconnect")]
    pub(crate) reconnect: bool,
    /// The first reconnect delay, doubled after each failed attempt.
    #[serde(default = "default_initial_delay_ms")]
    pub(crate) reconnect_initial_delay_ms: u64,
    /// The longest reconnect delay.
    #[serde(default = "default_max_delay_ms")]
    pub(crate) reconnect_max_delay_ms: u64,
    /// Reconnect attempts to the hub in use before turning to the failover
    /// hub, if any.
    #[serde(default = "default_max_retries")]
    pub(crate) reconnect_max_retries: u32,
}

fn default_timeout_ms() -> u64 {
    6000
}

fn default_reconnect() -> bool {
    true
}

fn default_initial_delay_ms() -> u64 {
    ScReconnectConfig::default().initial_delay_ms
}

fn default_max_delay_ms() -> u64 {
    ScReconnectConfig::default().max_delay_ms
}

fn default_max_retries() -> u32 {
    ScReconnectConfig::default().max_retries
}

/// The SC client's default connect timeout.
fn default_connect_timeout_ms() -> u64 {
    DEFAULT_SC_CONNECT_TIMEOUT_MS
}

impl Default for Sc {
    fn default() -> Self {
        Self {
            hub_url: String::new(),
            failover_hub_url: None,
            reconnect: default_reconnect(),
            reconnect_initial_delay_ms: default_initial_delay_ms(),
            reconnect_max_delay_ms: default_max_delay_ms(),
            reconnect_max_retries: default_max_retries(),
            connect_timeout_ms: default_connect_timeout_ms(),
            ca: PathBuf::new(),
            cert: PathBuf::new(),
            key: PathBuf::new(),
            vmac: None,
            device_uuid: None,
            timeout_ms: default_timeout_ms(),
            min_interval_ms: 0,
        }
    }
}

impl Sc {
    /// File-free checks: URIs, nonempty paths and request pacing.
    pub(crate) fn validate(&self) -> Result<(), String> {
        check_hub_url("Hub URL", &self.hub_url)?;
        if let Some(failover) = &self.failover_hub_url {
            check_hub_url("Failover URL", failover)?;
            if same_hub_url(failover, &self.hub_url) {
                return Err("Failover URL must differ from the primary hub URL".into());
            }
            if !self.reconnect {
                return Err(
                    "Failover URL needs reconnect on: failover happens by reconnecting".into(),
                );
            }
        }
        check_sc_connect_timeout(self.connect_timeout_ms)
            .map_err(|e| format!("connect_timeout_ms {e}"))?;
        self.reconnect_policy()
            .validate()
            .map_err(|e| format!("reconnect settings: {e}"))?;
        for (label, path) in [
            ("CA file", &self.ca),
            ("Certificate file", &self.cert),
            ("Key file", &self.key),
        ] {
            if path.as_os_str().is_empty() {
                return Err(format!("{label} path is required"));
            }
        }
        if self.min_interval_ms > bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS {
            return Err("min_interval_ms must be at most 3600000".into());
        }
        Ok(())
    }

    /// The backoff settings, whether or not `reconnect` is on.
    pub(crate) fn reconnect_policy(&self) -> ScReconnectConfig {
        ScReconnectConfig {
            initial_delay_ms: self.reconnect_initial_delay_ms,
            max_delay_ms: self.reconnect_max_delay_ms,
            max_retries: self.reconnect_max_retries,
        }
    }

    /// The reconnect policy the client gets: none when `reconnect` is off.
    pub(crate) fn reconnect(&self) -> Option<ScReconnectConfig> {
        self.reconnect.then(|| self.reconnect_policy())
    }

    /// Fill in a missing VMAC and device UUID. Returns whether anything
    /// changed, so the caller knows to persist it.
    pub(crate) fn assign_identity(&mut self) -> Result<bool, String> {
        let mut changed = false;
        if self.vmac.is_none() {
            let vmac = bacnet_transport::sc::generate_random48_vmac()
                .map_err(|e| format!("cannot generate a VMAC: {e}"))?;
            self.vmac = Some(Vmac(vmac));
            changed = true;
        }
        if self.device_uuid.is_none() {
            self.device_uuid = Some(DeviceUuid::generate()?);
            changed = true;
        }
        Ok(changed)
    }
}

/// A hub URI check with no I/O: `wss` scheme, a host, and a nonzero port
/// when one is given, with labelled messages; with `sc-tls`, also the
/// transport's own URL check. Catches a bad URI before anything is sent.
pub(crate) fn check_hub_url(label: &str, url: &str) -> Result<(), String> {
    let rest = url
        .get(..6)
        .filter(|scheme| scheme.eq_ignore_ascii_case("wss://"))
        .map(|_| &url[6..])
        .ok_or_else(|| format!("{label} must start with wss://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(format!("{label} must not carry a user name or password"));
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, tail) = v6
            .split_once(']')
            .ok_or_else(|| format!("{label} has an unclosed IPv6 literal"))?;
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(format!("{label} has an invalid IPv6 literal"));
        }
        match tail.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if tail.is_empty() => (host, None),
            None => return Err(format!("{label} has text after the IPv6 literal")),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("{label} must name a hub host"));
    }
    if let Some(port) = port {
        match port.parse::<u16>() {
            Ok(1..) => {}
            _ => return Err(format!("{label} port must be 1-65535")),
        }
    }
    // Then the dialer's own parse (scheme, host, TLS server name), so a URL
    // that passes here is one the transport accepts.
    #[cfg(feature = "sc-tls")]
    bacnet_transport::sc_tls::TlsWebSocket::validate_url(url)
        .map_err(|e| format!("{label} is not usable: {e}"))?;
    Ok(())
}

/// A VMAC, stored as colon-separated hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Vmac(pub(crate) [u8; 6]);

/// A device UUID, stored in RFC 4122 hyphenated form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceUuid(pub(crate) [u8; 16]);

impl DeviceUuid {
    /// A random (version 4) UUID from the OS generator.
    pub(crate) fn generate() -> Result<Self, String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| format!("cannot generate a device UUID: {e}"))?;
        bytes[6] = (bytes[6] & 0x0F) | 0x40;
        bytes[8] = (bytes[8] & 0x3F) | 0x80;
        Ok(Self(bytes))
    }
}

impl fmt::Display for Vmac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

impl fmt::Display for DeviceUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Vmac {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        parse_sc_vmac_arg(text).map(Self)
    }
}

impl std::str::FromStr for DeviceUuid {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        parse_sc_device_uuid_arg(text).map(Self)
    }
}

macro_rules! text_serde {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}
text_serde!(Vmac);
text_serde!(DeviceUuid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_urls_need_wss_a_host_and_a_nonzero_port() {
        for good in [
            "wss://hub.example.com",
            "WSS://hub.example.com:47808/.bacnet/sc?x=1",
            "wss://192.0.2.10:443",
            "wss://[2001:db8::1]:8443/hub",
            "wss://[::1]",
        ] {
            assert_eq!(check_hub_url("Hub URL", good), Ok(()), "{good}");
        }
        for (bad, why) in [
            ("ws://hub.example.com", "wss://"),
            ("https://hub.example.com", "wss://"),
            ("wss://", "host"),
            ("wss://:443", "host"),
            ("wss://hub:0", "1-65535"),
            ("wss://hub:65536", "1-65535"),
            ("wss://hub:http", "1-65535"),
            ("wss://user:pw@hub", "password"),
            ("wss://[::1", "unclosed"),
            ("wss://[zz]:1", "IPv6"),
            ("wss://hub name", "host"),
        ] {
            let error = check_hub_url("Hub URL", bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
    }

    #[test]
    fn generated_uuids_are_version_4_and_text_round_trips() {
        let a = DeviceUuid::generate().unwrap();
        let b = DeviceUuid::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.0[6] >> 4, 4);
        assert_eq!(a.0[8] >> 6, 0b10);
        let text = a.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(text.parse::<DeviceUuid>().unwrap(), a);
        let vmac = Vmac([0x22, 1, 2, 3, 4, 0xab]);
        assert_eq!(vmac.to_string(), "22:01:02:03:04:ab");
        assert_eq!("22:01:02:03:04:ab".parse::<Vmac>().unwrap(), vmac);
        assert!("ff:ff:ff:ff:ff:ff".parse::<Vmac>().is_err());
        assert!(
            "00000000-0000-0000-0000-000000000000"
                .parse::<DeviceUuid>()
                .is_err()
        );
    }

    #[test]
    fn identity_is_assigned_once() {
        let mut sc = Sc::default();
        assert!(sc.assign_identity().unwrap());
        let first = (sc.vmac, sc.device_uuid);
        assert!(!sc.assign_identity().unwrap());
        assert_eq!((sc.vmac, sc.device_uuid), first);
    }
}
