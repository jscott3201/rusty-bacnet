//! The Who-Is form: scope, target, instance range and listen time.
//!
//! A global or unbounded Who-Is can draw a reply from every device on a site,
//! so the first Enter shows a one-line warning with the expected volume and
//! only a second Enter sends it.

use std::time::Duration;

use crate::core::target::AddressStyle;
use crate::core::target::{parse_target, Target, MSTP_TEXT_PREFIX};
use crate::tui::message::{DirectedTo, WhoIsScope, WhoIsSpec, MAX_INSTANCE};
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::bip6::VMAC_TEXT_PREFIX;

/// Which scope the form's first field selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeChoice {
    /// Local broadcast.
    Local,
    /// Global broadcast.
    Global,
    /// Unicast to an address.
    Directed,
    /// Broadcast on a remote network.
    Network,
}

impl ScopeChoice {
    const ALL: [Self; 4] = [Self::Local, Self::Global, Self::Directed, Self::Network];

    fn step(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    /// Text shown in the form.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Local => "Local broadcast",
            Self::Global => "Global broadcast",
            Self::Directed => "Directed (one address)",
            Self::Network => "Remote network",
        }
    }

    /// True when the target field applies.
    pub(crate) fn needs_target(self) -> bool {
        matches!(self, Self::Directed | Self::Network)
    }
}

/// The form's fields, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    /// Scope selector.
    Scope,
    /// Address or network number.
    Target,
    /// Instance range.
    Range,
    /// Listen seconds.
    Listen,
}

/// A key the form understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormKey {
    /// Next field.
    Next,
    /// Previous field.
    Prev,
    /// Previous scope (on the scope field).
    Left,
    /// Next scope (on the scope field).
    Right,
    /// Type a character.
    Char(char),
    /// Delete the last character.
    Backspace,
    /// Send, or confirm a warning.
    Submit,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormOutcome {
    /// Still editing.
    Editing,
    /// Send this request.
    Send(WhoIsSpec),
}

/// Form state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhoIsForm {
    /// Selected scope.
    pub(crate) scope: ScopeChoice,
    /// Target text (directed address or network number).
    pub(crate) target: String,
    /// Range text: blank, `N`, or `LOW-HIGH`.
    pub(crate) range: String,
    /// Listen seconds text.
    pub(crate) listen: String,
    /// Focused field.
    pub(crate) focus: Field,
    /// Validation error from the last Enter.
    pub(crate) error: Option<String>,
    /// Warning waiting for a second Enter.
    pub(crate) warning: Option<String>,
}

impl Default for WhoIsForm {
    fn default() -> Self {
        Self {
            scope: ScopeChoice::Local,
            target: String::new(),
            range: String::new(),
            listen: "3".into(),
            focus: Field::Scope,
            error: None,
            warning: None,
        }
    }
}

impl WhoIsForm {
    /// A form pre-filled from the last request.
    pub(crate) fn from_spec(spec: &WhoIsSpec) -> Self {
        let (scope, target) = match &spec.scope {
            WhoIsScope::Local => (ScopeChoice::Local, String::new()),
            WhoIsScope::Global => (ScopeChoice::Global, String::new()),
            WhoIsScope::Directed { label, .. } => (ScopeChoice::Directed, label.clone()),
            WhoIsScope::Network(dnet) => (ScopeChoice::Network, dnet.to_string()),
        };
        let range = match spec.range {
            None => String::new(),
            Some(range) if range.low() == range.high() => range.low().to_string(),
            Some(range) => format!("{}-{}", range.low(), range.high()),
        };
        Self {
            scope,
            target,
            range,
            listen: spec.listen.as_secs().to_string(),
            ..Self::default()
        }
    }

    fn fields(&self) -> &'static [Field] {
        if self.scope.needs_target() {
            &[Field::Scope, Field::Target, Field::Range, Field::Listen]
        } else {
            &[Field::Scope, Field::Range, Field::Listen]
        }
    }

    fn step_focus(&mut self, forward: bool) {
        let fields = self.fields();
        let i = fields.iter().position(|&f| f == self.focus).unwrap_or(0);
        let n = fields.len();
        self.focus = fields[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }];
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            Field::Scope => None,
            Field::Target => Some(&mut self.target),
            Field::Range => Some(&mut self.range),
            Field::Listen => Some(&mut self.listen),
        }
    }

    /// Apply a key. `known` is how many devices the table holds, for the
    /// warning's volume estimate.
    pub(crate) fn key(&mut self, key: FormKey, style: AddressStyle, known: usize) -> FormOutcome {
        if key != FormKey::Submit {
            // Any edit withdraws a pending confirmation.
            self.warning = None;
        }
        match key {
            FormKey::Next => self.step_focus(true),
            FormKey::Prev => self.step_focus(false),
            FormKey::Left | FormKey::Right if self.focus == Field::Scope => {
                self.scope = self.scope.step(key == FormKey::Right);
            }
            FormKey::Left | FormKey::Right => {}
            FormKey::Char(' ') if self.focus == Field::Scope => {
                self.scope = self.scope.step(true);
            }
            FormKey::Char(ch) => {
                if let Some(text) = self.text_mut() {
                    if text.chars().count() < 48 && !ch.is_control() {
                        text.push(ch);
                    }
                }
            }
            FormKey::Backspace => {
                if let Some(text) = self.text_mut() {
                    text.pop();
                }
            }
            FormKey::Submit => return self.submit(style, known),
        }
        self.error = None;
        FormOutcome::Editing
    }

    fn submit(&mut self, style: AddressStyle, known: usize) -> FormOutcome {
        let spec = match self.validate(style) {
            Ok(spec) => spec,
            Err(error) => {
                self.error = Some(error);
                self.warning = None;
                return FormOutcome::Editing;
            }
        };
        self.error = None;
        match (self.warning.take(), warning_for(&spec, known)) {
            // Second Enter on an unchanged form: send.
            (Some(_), _) | (None, None) => FormOutcome::Send(spec),
            (None, Some(warning)) => {
                self.warning = Some(warning);
                FormOutcome::Editing
            }
        }
    }

    /// Check the fields and build the request.
    pub(crate) fn validate(&self, style: AddressStyle) -> Result<WhoIsSpec, String> {
        let target = self.target.trim();
        let scope = match self.scope {
            ScopeChoice::Local => WhoIsScope::Local,
            ScopeChoice::Global => WhoIsScope::Global,
            ScopeChoice::Directed => WhoIsScope::Directed {
                to: parse_directed(target, style)?,
                label: target.to_string(),
            },
            ScopeChoice::Network => WhoIsScope::Network(parse_network(target)?),
        };
        Ok(WhoIsSpec {
            scope,
            range: parse_range(self.range.trim())?,
            listen: parse_listen(self.listen.trim())?,
        })
    }
}

fn parse_directed(target: &str, style: AddressStyle) -> Result<DirectedTo, String> {
    if target.is_empty() {
        return Err("enter the address to send the Who-Is to".into());
    }
    if style == AddressStyle::Hex {
        if let Ok(vmac) = crate::transport::parse_sc_vmac_arg(target) {
            return Ok(DirectedTo::Mac(vmac.to_vec()));
        }
    }
    if target.starts_with(VMAC_TEXT_PREFIX) && style != AddressStyle::Bip6 {
        return Err(format!(
            "'{target}' is a B/IPv6 VMAC; bip6: addresses need the --ipv6 transport"
        ));
    }
    if target.starts_with(MSTP_TEXT_PREFIX) != (style == AddressStyle::Mstp) {
        return Err(format!(
            "'{target}' is not an address on this transport; a directed Who-Is needs {}",
            directed_forms(style)
        ));
    }
    match parse_target(target)? {
        Target::Mac(mac) => Ok(DirectedTo::Mac(mac)),
        // The worker finds the VMAC of the node at this address.
        Target::Bip6(peer) if style == AddressStyle::Bip6 => Ok(DirectedTo::Bip6(peer)),
        Target::Bip6(_) => Err(format!(
            "'{target}' is a B/IPv6 address; start the TUI with --ipv6"
        )),
        Target::Instance(_) | Target::Routed { .. } | Target::RoutedMac { .. } => Err(format!(
            "'{target}' is not an address; a directed Who-Is needs {}",
            directed_forms(style)
        )),
    }
}

/// The addresses a directed Who-Is takes on a transport with `style`.
pub(crate) fn directed_forms(style: AddressStyle) -> &'static str {
    match style {
        AddressStyle::Bip => "IP or IP:port",
        AddressStyle::Bip6 => "bip6:N, bip6:0xVMAC or [IPv6]:port",
        AddressStyle::Hex => "VMAC as 12 hex digits",
        AddressStyle::Mstp => "mstp:N, a station 0 to 254",
    }
}

fn parse_network(target: &str) -> Result<u16, String> {
    match target.parse::<u16>() {
        Ok(dnet @ 1..=65_534) => Ok(dnet),
        Ok(65_535) => Err("network 65535 is the global broadcast; choose Global scope".into()),
        _ => Err(format!("'{target}' is not a network number (1-65534)")),
    }
}

/// Blank is unbounded; `N` is one instance; `LOW-HIGH` uses the shared parser,
/// which refuses one limit alone, a low limit above the high one and a limit
/// past the highest instance (#1483).
fn parse_range(text: &str) -> Result<Option<DeviceInstanceRange>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    if text.contains('-') {
        return crate::core::range::parse_discover_range(Some(text)).map_err(|e| e.to_string());
    }
    let n = text
        .parse::<u32>()
        .map_err(|_| format!("invalid instance range '{text}': use N or LOW-HIGH"))?;
    DeviceInstanceRange::single(n)
        .map(Some)
        .map_err(|_| format!("instances run from 0 to {MAX_INSTANCE}"))
}

fn parse_listen(text: &str) -> Result<Duration, String> {
    match text.parse::<u64>() {
        Ok(secs @ 1..=60) => Ok(Duration::from_secs(secs)),
        _ => Err("listen for 1 to 60 seconds".into()),
    }
}

/// The one-line warning for a global or unbounded Who-Is, or `None`. A
/// directed Who-Is goes to one address, so it never warns.
pub(crate) fn warning_for(spec: &WhoIsSpec, known: usize) -> Option<String> {
    if matches!(spec.scope, WhoIsScope::Directed { .. }) {
        return None;
    }
    let volume = match known {
        0 => String::new(),
        1 => " (1 device known)".to_string(),
        n => format!(" ({n} devices known)"),
    };
    let text = match (spec.is_global(), spec.is_full_range()) {
        (true, true) => {
            format!("Global Who-Is, no range: every device on every network answers{volume}.")
        }
        (true, false) => {
            format!("Global Who-Is: matching devices on every network answer{volume}.")
        }
        (false, true) => format!("No instance range: every device in scope answers{volume}."),
        (false, false) => return None,
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(form: &mut WhoIsForm, text: &str) {
        for ch in text.chars() {
            form.key(FormKey::Char(ch), AddressStyle::Bip, 0);
        }
    }

    #[test]
    fn bounded_local_who_is_sends_on_first_enter() {
        let mut form = WhoIsForm::default();
        form.key(FormKey::Next, AddressStyle::Bip, 0);
        assert_eq!(form.focus, Field::Range, "local scope skips the target");
        type_text(&mut form, "101-103");
        let FormOutcome::Send(spec) = form.key(FormKey::Submit, AddressStyle::Bip, 5) else {
            panic!("expected Send");
        };
        assert_eq!(spec.scope, WhoIsScope::Local);
        assert_eq!(
            spec.range,
            Some(DeviceInstanceRange::new(101, 103).unwrap())
        );
        assert_eq!(spec.listen, Duration::from_secs(3));
    }

    #[test]
    fn unbounded_or_global_who_is_needs_a_second_enter() {
        let mut form = WhoIsForm::default();
        assert_eq!(
            form.key(FormKey::Submit, AddressStyle::Bip, 7),
            FormOutcome::Editing
        );
        let warning = form.warning.clone().expect("warning shown");
        assert!(warning.contains("every device") && warning.contains("7 devices known"));
        assert!(matches!(
            form.key(FormKey::Submit, AddressStyle::Bip, 7),
            FormOutcome::Send(_)
        ));

        let mut global = WhoIsForm::default();
        global.key(FormKey::Right, AddressStyle::Bip, 0);
        assert_eq!(global.scope, ScopeChoice::Global);
        global.key(FormKey::Next, AddressStyle::Bip, 0);
        type_text(&mut global, "5-6");
        assert_eq!(
            global.key(FormKey::Submit, AddressStyle::Bip, 0),
            FormOutcome::Editing
        );
        assert!(global
            .warning
            .as_deref()
            .unwrap()
            .starts_with("Global Who-Is"));
        // An edit withdraws the pending confirmation.
        global.key(FormKey::Backspace, AddressStyle::Bip, 0);
        assert_eq!(global.warning, None);
    }

    #[test]
    fn a_directed_who_is_without_a_range_sends_on_the_first_enter() {
        let mut form = WhoIsForm {
            scope: ScopeChoice::Directed,
            target: "10.0.0.7:47809".into(),
            ..WhoIsForm::default()
        };
        let FormOutcome::Send(spec) = form.key(FormKey::Submit, AddressStyle::Bip, 9) else {
            panic!("a directed Who-Is reaches one address and must not warn");
        };
        assert_eq!(spec.range, None);
        assert_eq!(warning_for(&spec, 9), None);
    }

    #[test]
    fn directed_and_network_targets_are_validated() {
        let mut form = WhoIsForm {
            scope: ScopeChoice::Directed,
            target: "10.0.0.7".into(),
            range: "42".into(),
            ..WhoIsForm::default()
        };
        let FormOutcome::Send(spec) = form.key(FormKey::Submit, AddressStyle::Bip, 0) else {
            panic!("expected Send");
        };
        assert_eq!(
            spec.scope,
            WhoIsScope::Directed {
                to: DirectedTo::Mac(vec![10, 0, 0, 7, 0xBA, 0xC0]),
                label: "10.0.0.7".into()
            }
        );
        assert_eq!(spec.range, Some(DeviceInstanceRange::single(42).unwrap()));

        form.target = "1234".into();
        form.key(FormKey::Submit, AddressStyle::Bip, 0);
        assert!(form.error.as_deref().unwrap().contains("not an address"));
        // An IPv6 address needs the B/IPv6 transport, which resolves it.
        form.target = "[::1]:47808".into();
        form.key(FormKey::Submit, AddressStyle::Bip, 0);
        assert!(form.error.as_deref().unwrap().contains("--ipv6"));
        // So does a B/IPv6 VMAC, which B/IP and SC can't send to.
        for style in [AddressStyle::Bip, AddressStyle::Hex] {
            form.target = "bip6:1234".into();
            form.key(FormKey::Submit, style, 0);
            assert!(form
                .error
                .as_deref()
                .unwrap()
                .contains("bip6: addresses need the --ipv6 transport"));
        }

        let mut sc = form.clone();
        sc.target = "020000000009".into();
        assert!(matches!(
            sc.key(FormKey::Submit, AddressStyle::Hex, 0),
            FormOutcome::Send(WhoIsSpec {
                scope: WhoIsScope::Directed { .. },
                ..
            })
        ));

        // B/IPv6: a VMAC as the table prints it, or an address the worker
        // resolves to one.
        let mut bip6 = form.clone();
        for (target, to) in [
            ("bip6:0x0004D2", DirectedTo::Mac(vec![0, 4, 0xD2])),
            ("bip6:1234", DirectedTo::Mac(vec![0, 4, 0xD2])),
            (
                "[::1]:47809",
                DirectedTo::Bip6("[::1]:47809".parse().unwrap()),
            ),
        ] {
            bip6.target = target.into();
            let FormOutcome::Send(spec) = bip6.key(FormKey::Submit, AddressStyle::Bip6, 0) else {
                panic!("expected Send for {target}");
            };
            assert_eq!(
                spec.scope,
                WhoIsScope::Directed {
                    to,
                    label: target.into()
                }
            );
        }
        bip6.target = "1234".into();
        bip6.key(FormKey::Submit, AddressStyle::Bip6, 0);
        assert!(bip6.error.as_deref().unwrap().contains("bip6:N"));

        form.scope = ScopeChoice::Network;
        for (target, expected) in [("0", "not a network"), ("65535", "global broadcast")] {
            form.target = target.into();
            form.key(FormKey::Submit, AddressStyle::Bip, 0);
            assert!(
                form.error.as_deref().unwrap().contains(expected),
                "{target}"
            );
        }
    }

    #[test]
    fn range_and_listen_limits() {
        assert_eq!(
            parse_range("0-4194303"),
            Ok(Some(DeviceInstanceRange::new(0, MAX_INSTANCE).unwrap()))
        );
        assert!(parse_range("0-4194304")
            .unwrap_err()
            .contains("above the highest instance"));
        assert_eq!(
            parse_range("4194303"),
            Ok(Some(DeviceInstanceRange::single(MAX_INSTANCE).unwrap()))
        );
        assert!(parse_range("4194304")
            .unwrap_err()
            .contains("instances run from 0 to 4194303"));
        assert!(parse_range("9-1")
            .unwrap_err()
            .contains("low (9) > high (1)"));
        // One limit alone is refused, not sent as an unbounded Who-Is (#1483).
        assert!(parse_range("9-")
            .unwrap_err()
            .contains("a low limit but no high limit"));
        assert!(parse_listen("0").is_err() && parse_listen("61").is_err());
        let spec = WhoIsForm {
            range: "0-4194303".into(),
            ..WhoIsForm::default()
        }
        .validate(AddressStyle::Bip)
        .unwrap();
        assert!(
            warning_for(&spec, 0).is_some(),
            "the explicit full range warns too"
        );
    }

    #[test]
    fn from_spec_round_trips_the_last_request() {
        let spec = WhoIsSpec {
            scope: WhoIsScope::Network(7),
            range: Some(DeviceInstanceRange::new(10, 20).unwrap()),
            listen: Duration::from_secs(5),
        };
        let form = WhoIsForm::from_spec(&spec);
        assert_eq!(form.validate(AddressStyle::Bip), Ok(spec));
    }
}
