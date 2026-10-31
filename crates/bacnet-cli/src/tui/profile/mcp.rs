//! A profile's `[mcp]` table: what `bacnet mcp serve` may reach on the
//! site, and how fast (#2066). Every key is optional; an absent table lets
//! the server reach every network and device at the default rates, with no
//! broad discovery.
//!
//! ```toml
//! [mcp]
//! networks = [0, 2001]          # 0 is the local network
//! devices = ["1000-1999", "5"]
//! slow_networks = [2001]        # MS/TP behind a router: 2 requests a second
//! broad_discovery = false
//! mcp_writes = "elicit"         # offer bacnet_write and bacnet_revert (#2068)
//! mcp_arm = "elicit"            # the client's confirmation may arm a session
//!
//! [mcp.plan]                    # writes approved in advance, no prompt
//! devices = ["1201"]
//! properties = ["present-value"]
//! priority_min = 8
//! max_writes = 50
//! hours = 4                     # from `bacnet mcp authorize`, 8 at most
//! ```
//!
//! The TUI uses the table for its local MCP socket (#2185): while the
//! profile is open, agents may attach through `bacnet mcp connect`, under
//! the table's reach and rates and read-only, unless `tui_listener =
//! false`. The table's first-use
//! authorization, separate from the TUI's, is recorded in it by `bacnet mcp
//! authorize` (`authorized_at_unix`, and the reach it covered as
//! `[mcp.authorized]`) and cleared when the table widens the server's
//! reach.

use serde::{Deserialize, Serialize};

/// The `[mcp]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpSection {
    /// Networks the server may reach (0 for the local one); absent for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) networks: Option<Vec<u16>>,
    /// Device instances it may reach, as `low-high` or one instance;
    /// absent for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) devices: Option<Vec<String>>,
    /// Networks held to `slow_rate`, such as a 9,600-baud MS/TP trunk.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) slow_networks: Vec<u16>,
    /// Networks held to `fast_rate`, such as a fast MS/TP trunk.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) fast_networks: Vec<u16>,
    /// Whether a global or unbounded Who-Is may go, one a minute.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) broad_discovery: bool,
    /// Requests a second across everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_rate: Option<u32>,
    /// Requests a second to any one network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_rate: Option<u32>,
    /// Requests a second to a slow network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slow_rate: Option<u32>,
    /// Requests a second to a fast MS/TP network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fast_rate: Option<u32>,
    /// Requests a second for the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) principal_rate: Option<u32>,
    /// Requests the agent may send at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) principal_burst: Option<u32>,
    /// Property values one read returns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_values: Option<usize>,
    /// Devices on a discovery page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page_devices: Option<usize>,
    /// Objects on an object list page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page_objects: Option<usize>,
    /// Characters kept of a device-supplied string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_text: Option<usize>,
    /// Seconds a call may wait on the limits and the network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) call_budget_s: Option<u64>,
    /// Whether `bacnet_write` and `bacnet_revert` are offered (#2068):
    /// `never` (the default) or `elicit`, the client's confirmation
    /// approving T1 writes only.
    #[serde(skip_serializing_if = "McpChoice::is_never")]
    pub(crate) mcp_writes: McpChoice,
    /// Whether the client's confirmation may arm a write session:
    /// `never` (the default) or `elicit`.
    #[serde(skip_serializing_if = "McpChoice::is_never")]
    pub(crate) mcp_arm: McpChoice,
    /// Writes approved in advance, by `bacnet mcp authorize`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan: Option<McpPlan>,
    /// Whether the TUI listens for agents on its local socket while the
    /// profile is open (#2185); absent, it does whenever MCP is
    /// authorized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tui_listener: Option<bool>,
    /// When `bacnet mcp authorize` recorded consent to serve MCP on this
    /// profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_at_unix: Option<u64>,
    /// The reach that consent covered (this table without its own
    /// consent); a table that widens it needs authorizing again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorized: Option<Box<McpSection>>,
    /// The profile's transport settings that consent covered, as written
    /// by [`Profile::link`](super::Profile::link); any change to them needs
    /// authorizing again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_transport: Option<String>,
}

/// `mcp_writes` and `mcp_arm`: `never` or `elicit`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum McpChoice {
    /// Not through MCP: the default.
    #[default]
    Never,
    /// Through the client's confirmation (an elicitation form).
    Elicit,
}

impl McpChoice {
    fn is_never(&self) -> bool {
        *self == Self::Never
    }
}

/// A plan's device ranges and properties, parsed.
pub(crate) type PlanTargets = (
    Vec<(u32, u32)>,
    Vec<bacnet_types::enums::PropertyIdentifier>,
);

/// `[mcp.plan]`: a commissioning plan. Writes it covers run without a
/// prompt for `hours` after `bacnet mcp authorize`, up to `max_writes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpPlan {
    /// Devices it covers, as `low-high` or one instance.
    pub(crate) devices: Vec<String>,
    /// Properties it may write, by name.
    #[serde(default = "McpPlan::present_value")]
    pub(crate) properties: Vec<String>,
    /// The lowest priority number it may command (8 by default).
    #[serde(default = "McpPlan::eight")]
    pub(crate) priority_min: u8,
    /// The highest (16 by default).
    #[serde(default = "McpPlan::sixteen")]
    pub(crate) priority_max: u8,
    /// How many writes it allows.
    pub(crate) max_writes: u32,
    /// How long it lasts after authorization, 1 to 8 hours.
    pub(crate) hours: u64,
    /// Why, for the audit log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

impl McpPlan {
    fn present_value() -> Vec<String> {
        vec!["present-value".into()]
    }
    fn eight() -> u8 {
        8
    }
    fn sixteen() -> u8 {
        16
    }

    /// The device ranges, and the properties, parsed and checked.
    pub(crate) fn parsed(&self) -> Result<PlanTargets, String> {
        if !(1..=8).contains(&self.hours) {
            return Err("[mcp.plan] hours must be 1 to 8".into());
        }
        let devices = self
            .devices
            .iter()
            .map(|text| device_range(text))
            .collect::<Result<Vec<_>, _>>()?;
        let properties = self
            .properties
            .iter()
            .map(|text| match bacnet_ops::parse::parse_property(text) {
                Ok((property, None)) => Ok(property),
                _ => Err(format!("[mcp.plan] properties: '{text}' is not a property")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((devices, properties))
    }
}

/// The rates an absent key means, in [`McpSection::rates`] order: the MCP
/// server's defaults (`bacnet_mcp::policy::Rates`; a test of the `mcp`
/// feature checks they agree).
pub(crate) const DEFAULT_RATES: [u32; 6] = [20, 20, 2, 5, 5, 20];

/// The caps an absent key means, in [`McpSection::caps`] order
/// (`bacnet_mcp::policy::Caps`).
pub(crate) const DEFAULT_CAPS: [u64; 5] = [100, 200, 200, 256, 30];

/// One `devices` entry: `low-high` or one instance.
pub(crate) fn device_range(text: &str) -> Result<(u32, u32), String> {
    let bad = || format!("[mcp] devices: '{text}' is not low-high or a device instance");
    let (low, high) = match text.split_once('-') {
        Some((low, high)) => (
            low.trim().parse().map_err(|_| bad())?,
            high.trim().parse().map_err(|_| bad())?,
        ),
        None => {
            let one = text.trim().parse().map_err(|_| bad())?;
            (one, one)
        }
    };
    if low > high || high > 4_194_303 {
        return Err(bad());
    }
    Ok((low, high))
}

impl McpSection {
    /// Check what the TUI can check without the MCP server: the device
    /// ranges parse. The server checks the rest when it starts.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for text in self.devices.iter().flatten() {
            device_range(text)?;
        }
        if let Some(plan) = &self.plan {
            plan.parsed()?;
            if self.mcp_writes == McpChoice::Never {
                return Err("[mcp.plan] needs mcp_writes = \"elicit\"".into());
            }
        }
        if self.mcp_arm == McpChoice::Elicit && self.mcp_writes == McpChoice::Never {
            return Err("[mcp] mcp_arm = \"elicit\" needs mcp_writes = \"elicit\"".into());
        }
        Ok(())
    }

    /// The device ranges, parsed.
    fn ranges(&self) -> Option<Vec<(u32, u32)>> {
        self.devices.as_ref().map(|list| {
            list.iter()
                .filter_map(|text| device_range(text).ok())
                .collect()
        })
    }

    /// Whether consent covers this table now: a time, and the table and
    /// transport it covered. A time alone, as a hand edit might leave,
    /// authorizes nothing.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn authorized(&self) -> bool {
        self.authorized_at_unix.is_some()
            && self.authorized.is_some()
            && self.authorized_transport.is_some()
    }

    /// Whether the TUI opens its MCP socket on this profile: only while
    /// consent stands, and then unless `tui_listener = false`.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn listens(&self) -> bool {
        self.authorized() && self.tui_listener != Some(false)
    }

    /// Record consent, given at `at`, to this table and to the transport
    /// settings `link`.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn authorize(&mut self, at: u64, link: String) {
        let mut covered = self.clone();
        covered.authorized_at_unix = None;
        covered.authorized = None;
        covered.authorized_transport = None;
        self.authorized_at_unix = Some(at);
        self.authorized = Some(Box::new(covered));
        self.authorized_transport = Some(link);
    }

    /// Drop a consent that doesn't cover this table, or the transport
    /// settings `link`, any more.
    pub(crate) fn revoke_if_raised(&mut self, link: &str) {
        let stands = match (&self.authorized, &self.authorized_transport) {
            (Some(covered), Some(transport)) => transport == link && !self.raises(covered),
            _ => false,
        };
        if !stands {
            self.authorized_at_unix = None;
            self.authorized = None;
            self.authorized_transport = None;
        }
    }

    /// The rates, defaults filled in: max, network, slow, fast, principal,
    /// burst.
    fn rates(&self) -> [u32; 6] {
        let keys = [
            self.max_rate,
            self.network_rate,
            self.slow_rate,
            self.fast_rate,
            self.principal_rate,
            self.principal_burst,
        ];
        std::array::from_fn(|i| keys[i].unwrap_or(DEFAULT_RATES[i]))
    }

    /// The caps, defaults filled in: values, devices and objects a page,
    /// text characters, call budget seconds.
    fn caps(&self) -> [u64; 5] {
        let size = |key: Option<usize>| key.map(|n| u64::try_from(n).unwrap_or(u64::MAX));
        let keys = [
            size(self.max_values),
            size(self.page_devices),
            size(self.page_objects),
            size(self.max_text),
            self.call_budget_s,
        ];
        std::array::from_fn(|i| keys[i].unwrap_or(DEFAULT_CAPS[i]))
    }

    /// True when `self` lets the server do more than `from` did: reach a
    /// network or device outside `from`'s lists, discover broadly, send
    /// faster (a higher rate, or a network no longer held to its slow or
    /// fast rate), or do more in one call (a higher cap).
    pub(crate) fn raises(&self, from: &Self) -> bool {
        let faster = self
            .rates()
            .iter()
            .zip(from.rates())
            .any(|(now, was)| *now > was);
        let bigger = self
            .caps()
            .iter()
            .zip(from.caps())
            .any(|(now, was)| *now > was);
        let unpaced = from
            .slow_networks
            .iter()
            .any(|n| !self.slow_networks.contains(n))
            || from
                .fast_networks
                .iter()
                .any(|n| !self.fast_networks.contains(n))
            || (self.rates()[3] > self.rates()[1]
                && self
                    .fast_networks
                    .iter()
                    .any(|n| !from.fast_networks.contains(n)))
            || (self.rates()[2] > self.rates()[1]
                && self
                    .slow_networks
                    .iter()
                    .any(|n| !from.slow_networks.contains(n)));
        // Offering writes, arming by the client, or any change to a plan
        // grant (#2068).
        let writes = (self.mcp_writes == McpChoice::Elicit && from.mcp_writes == McpChoice::Never)
            || (self.mcp_arm == McpChoice::Elicit && from.mcp_arm == McpChoice::Never)
            || (self.plan.is_some() && self.plan != from.plan);
        // The TUI's socket, where consent covered a table that turned it off.
        let listener = self.tui_listener != Some(false) && from.tui_listener == Some(false);
        faster || bigger || unpaced || writes || listener || self.reaches_further(from)
    }

    /// True when `self` reaches a network or device outside `from`'s
    /// lists, or discovers broadly where `from` didn't.
    fn reaches_further(&self, from: &Self) -> bool {
        let networks = match (&self.networks, &from.networks) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(now), Some(was)) => now.iter().any(|n| !was.contains(n)),
        };
        let devices = match (self.ranges(), from.ranges()) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(now), Some(was)) => now
                .iter()
                .any(|&(low, high)| !was.iter().any(|&(l, h)| l <= low && high <= h)),
        };
        networks || devices || (self.broad_discovery && !from.broad_discovery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_and_round_trips() {
        let text = "networks = [0, 2001]\ndevices = [\"1000-1999\", \"5\"]\n\
                    slow_networks = [2001]\nmax_rate = 10\n";
        let section: McpSection = toml::from_str(text).unwrap();
        section.validate().unwrap();
        assert_eq!(section.networks, Some(vec![0, 2001]));
        assert_eq!(section.max_rate, Some(10));
        let again: McpSection = toml::from_str(&toml::to_string(&section).unwrap()).unwrap();
        assert_eq!(again, section);
        assert!(toml::from_str::<McpSection>("writes = true").is_err());
        let bad = McpSection {
            devices: Some(vec!["9-1".into()]),
            ..McpSection::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn widening_reach_raises_and_narrowing_does_not() {
        let narrow = McpSection {
            networks: Some(vec![0]),
            devices: Some(vec!["100-199".into()]),
            ..McpSection::default()
        };
        let all = McpSection::default();
        assert!(
            !narrow.raises(&all),
            "narrowing never needs a new authorization"
        );
        let faster = McpSection {
            max_rate: Some(21),
            ..narrow.clone()
        };
        assert!(faster.raises(&narrow), "a higher rate");
        let longer = McpSection {
            call_budget_s: Some(31),
            ..narrow.clone()
        };
        assert!(longer.raises(&narrow), "a longer budget");
        let more_values = McpSection {
            max_values: Some(101),
            ..narrow.clone()
        };
        assert!(more_values.raises(&narrow), "a higher cap");
        let paced = McpSection {
            slow_networks: vec![7],
            ..narrow.clone()
        };
        assert!(narrow.raises(&paced), "a network no longer slow");
        assert!(!paced.raises(&narrow), "a network newly slow");
        let quick_slow = McpSection {
            slow_networks: vec![7],
            slow_rate: Some(50),
            ..narrow.clone()
        };
        let quick_base = McpSection {
            slow_rate: Some(50),
            ..narrow.clone()
        };
        assert!(
            quick_slow.raises(&quick_base),
            "a network newly \"slow\" at a rate above the network rate goes faster"
        );
        assert!(all.raises(&narrow));
        let more_networks = McpSection {
            networks: Some(vec![0, 7]),
            ..narrow.clone()
        };
        assert!(more_networks.raises(&narrow));
        let more_devices = McpSection {
            devices: Some(vec!["100-250".into()]),
            ..narrow.clone()
        };
        assert!(more_devices.raises(&narrow));
        let broad = McpSection {
            broad_discovery: true,
            ..narrow.clone()
        };
        assert!(broad.raises(&narrow));
        let slower = McpSection {
            max_rate: Some(1),
            ..narrow.clone()
        };
        assert!(!slower.raises(&narrow));
    }

    /// The TUI listens by default once consent stands, never without it,
    /// and turning the listener back on needs consent again (#2185).
    #[test]
    fn the_tui_listener_follows_consent_and_its_key() {
        let mut section: McpSection = toml::from_str("networks = [0]\n").unwrap();
        assert_eq!(section.tui_listener, None);
        assert!(!section.listens(), "never without consent");
        section.authorize(1, "bip".into());
        assert!(section.listens(), "on by default with consent");
        let off: McpSection = toml::from_str("tui_listener = false\n").unwrap();
        let mut off_authorized = off.clone();
        off_authorized.authorize(1, "bip".into());
        assert!(!off_authorized.listens());
        let on = McpSection {
            tui_listener: Some(true),
            ..off.clone()
        };
        assert!(on.raises(&off) && McpSection::default().raises(&off));
        assert!(!off.raises(&on) && !on.raises(&McpSection::default()));
        let again: McpSection = toml::from_str(&toml::to_string(&off).unwrap()).unwrap();
        assert_eq!(again, off);
    }
}
