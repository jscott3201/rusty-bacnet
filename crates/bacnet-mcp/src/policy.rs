//! What the server may reach, and how fast: the launch configuration a
//! front end builds from a profile's `[mcp]` table or its flags (design
//! §4.4, §7.6). It is fixed for the life of the server, so the tool list
//! never depends on runtime state.

use std::collections::BTreeSet;
use std::time::Duration;

use bacnet_ops::limits::{Overlay, RequestLimits};

mod writes;

pub use writes::{
    ArmMode, MAX_PLAN_SECS, MAX_PLAN_WRITES, MAX_WRITES, PlanGrant, Workspace, WriteMode,
    WritePolicy,
};

/// The server's launch policy. [`Policy::default`] is the read-only
/// default every deployment starts from: every network and device, no
/// broad discovery, the default rates and caps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Never transmit: no client is built, and only the tools that send
    /// nothing are listed.
    pub passive: bool,
    /// The networks the tools may reach, 0 being the local one; `None`
    /// for every network.
    pub networks: Option<BTreeSet<u16>>,
    /// The device instances the tools may reach, as inclusive ranges;
    /// `None` for every device.
    pub devices: Option<Vec<(u32, u32)>>,
    /// Networks held to [`Rates::slow`]: a router doesn't say what link it
    /// reaches, so a slow MS/TP trunk has to be named.
    pub slow_networks: BTreeSet<u16>,
    /// Networks held to [`Rates::fast`], such as an MS/TP trunk at
    /// 76,800 baud or more.
    pub fast_networks: BTreeSet<u16>,
    /// Whether a global or full-range Who-Is may be sent, at most one a
    /// minute.
    pub broad_discovery: bool,
    /// The request rates.
    pub rates: Rates,
    /// The per-call caps.
    pub caps: Caps,
    /// What the server may change: no write tool by default.
    pub writes: WritePolicy,
}

/// Requests a second, at each layer (design §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rates {
    /// Across every device and network.
    pub global: u32,
    /// For any network without a rate of its own.
    pub network: u32,
    /// For the networks in [`Policy::slow_networks`].
    pub slow: u32,
    /// For the networks in [`Policy::fast_networks`].
    pub fast: u32,
    /// For each principal (agent), refilled at this rate.
    pub principal: u32,
    /// How many requests a principal may send at once.
    pub burst: u32,
}

impl Default for Rates {
    fn default() -> Self {
        Self {
            global: 20,
            network: 20,
            slow: 2,
            fast: 5,
            principal: 5,
            burst: 20,
        }
    }
}

/// What one tool call may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Property values one `bacnet_read` returns.
    pub values: usize,
    /// Devices on one `bacnet_discover` page.
    pub devices: usize,
    /// Objects on one `bacnet_list_objects` page.
    pub objects: usize,
    /// Characters kept of a device-supplied string.
    pub text: usize,
    /// How long a call may wait for the limits and the network before it
    /// returns what it has.
    pub budget: Duration,
    /// Points one principal may watch at once, through subscriptions and
    /// `bacnet_watch` together (design §7.6).
    pub watches: usize,
    /// Points the whole process may watch at once: the watch hub's cap.
    pub process_watches: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            values: 100,
            devices: 200,
            objects: 200,
            text: 256,
            budget: Duration::from_secs(30),
            watches: 32,
            process_watches: 256,
        }
    }
}

/// Bounds the validation holds the caps to.
const MAX_VALUES: usize = 1_000;
const MAX_PAGE: usize = 2_000;
const MAX_TEXT: usize = 4_096;
const MAX_BUDGET: Duration = Duration::from_secs(600);
/// A device's COV table is small; no process watches more than this.
const MAX_WATCHES: usize = 1_024;

impl Policy {
    /// Check the policy before anything starts: a bad configuration
    /// refuses to start rather than run with a guess.
    pub fn validate(&self) -> Result<(), String> {
        self.writes.validate()?;
        if let Some(networks) = &self.networks
            && networks.contains(&u16::MAX)
        {
            return Err("networks: 65535 is the global broadcast, not a network".into());
        }
        if let Some(devices) = &self.devices {
            for &(low, high) in devices {
                if low > high || high > bacnet_types::primitives::ObjectIdentifier::MAX_INSTANCE {
                    return Err(format!(
                        "devices: {low}-{high} is not a range of instances 0 to 4194303"
                    ));
                }
            }
        }
        if let Some(both) = self.slow_networks.intersection(&self.fast_networks).next() {
            return Err(format!(
                "network {both} is both in slow_networks and fast_networks"
            ));
        }
        let rates = &self.rates;
        for (name, value) in [
            ("max_rate", rates.global),
            ("network_rate", rates.network),
            ("slow_rate", rates.slow),
            ("fast_rate", rates.fast),
            ("principal_rate", rates.principal),
            ("principal_burst", rates.burst),
        ] {
            if value == 0 || value > 1_000 {
                return Err(format!("{name} must be 1 to 1000, not {value}"));
            }
        }
        let caps = &self.caps;
        if !(1..=MAX_VALUES).contains(&caps.values) {
            return Err(format!("max_values must be 1 to {MAX_VALUES}"));
        }
        if !(1..=MAX_PAGE).contains(&caps.devices) || !(1..=MAX_PAGE).contains(&caps.objects) {
            return Err(format!("page sizes must be 1 to {MAX_PAGE}"));
        }
        if !(16..=MAX_TEXT).contains(&caps.text) {
            return Err(format!("max_text must be 16 to {MAX_TEXT} characters"));
        }
        if caps.budget.is_zero() || caps.budget > MAX_BUDGET {
            return Err("call_budget_s must be 1 to 600".into());
        }
        if !(1..=MAX_WATCHES).contains(&caps.process_watches)
            || !(1..=caps.process_watches).contains(&caps.watches)
        {
            return Err(format!(
                "watches must be 1 to the process cap, and the process cap 1 to {MAX_WATCHES}"
            ));
        }
        Ok(())
    }

    /// Whether the tools may reach `network` (0 for the local one).
    pub fn allows_network(&self, network: u16) -> bool {
        self.networks
            .as_ref()
            .is_none_or(|networks| networks.contains(&network))
    }

    /// Whether the tools may reach device `instance`.
    pub fn allows_device(&self, instance: u32) -> bool {
        self.devices.as_ref().is_none_or(|ranges| {
            ranges
                .iter()
                .any(|&(low, high)| (low..=high).contains(&instance))
        })
    }

    /// The request limits the engine runs under: the global rate, and a
    /// rate per network.
    pub fn request_limits(&self) -> RequestLimits {
        let overlay = self.overlay();
        RequestLimits::new(overlay.global).with_network_rates(overlay.network, overlay.rates)
    }

    /// The same rates as an overlay on another front end's limits: the
    /// TUI's while an agent is attached to it (#2185).
    pub fn overlay(&self) -> Overlay {
        let rates = self.rates;
        let slow = self.slow_networks.iter().map(|&n| (n, rates.slow));
        let fast = self.fast_networks.iter().map(|&n| (n, rates.fast));
        Overlay {
            global: rates.global,
            network: rates.network,
            rates: slow.chain(fast).collect(),
        }
    }

    /// The fastest a request to `network` may go, for the limits block.
    pub fn network_rate(&self, network: u16) -> u32 {
        if self.slow_networks.contains(&network) {
            self.rates.slow
        } else if self.fast_networks.contains(&network) {
            self.rates.fast
        } else {
            self.rates.network
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_reaches_everything_and_validates() {
        let policy = Policy::default();
        policy.validate().unwrap();
        assert!(policy.allows_network(0) && policy.allows_network(2001));
        assert!(policy.allows_device(0) && policy.allows_device(4_194_302));
        assert_eq!(policy.network_rate(7), 20);
    }

    #[test]
    fn lists_limit_networks_and_devices() {
        let policy = Policy {
            networks: Some([0, 2001].into()),
            devices: Some(vec![(1000, 1999), (5, 5)]),
            slow_networks: [2001].into(),
            ..Policy::default()
        };
        policy.validate().unwrap();
        assert!(policy.allows_network(2001) && !policy.allows_network(2002));
        assert!(policy.allows_device(5) && policy.allows_device(1999));
        assert!(!policy.allows_device(6) && !policy.allows_device(2000));
        assert_eq!(policy.network_rate(2001), 2);
    }

    #[test]
    fn a_bad_policy_refuses_to_start() {
        let bad = [
            Policy {
                devices: Some(vec![(9, 1)]),
                ..Policy::default()
            },
            Policy {
                slow_networks: [5].into(),
                fast_networks: [5].into(),
                ..Policy::default()
            },
            Policy {
                rates: Rates {
                    principal: 0,
                    ..Rates::default()
                },
                ..Policy::default()
            },
            Policy {
                networks: Some([65535].into()),
                ..Policy::default()
            },
        ];
        for policy in bad {
            assert!(policy.validate().is_err(), "{policy:?}");
        }
    }
}
