//! The profile picker, editor and resolved preview. No filesystem or network I/O.

use crate::tui::message::Command;
use crate::tui::profile::checks::Checks;
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile, Transport, Writes};
use std::path::PathBuf;

/// Editing keys have no transport effects until a request is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Next,
    Prev,
    Char(char),
    Backspace,
    Discard,
    Submit,
    /// Start a new B/IP profile.
    New,
    /// Start a new BACnet/SC profile.
    NewSc,
    /// Start a new MS/TP profile.
    NewMstp,
    Edit,
    Reload,
    List,
    Connect,
    Attest,
    /// Accept the preview's check warnings for the next connection.
    Confirm,
}

/// One of the connection dialog's three screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Edit,
    Preview,
}

/// B/IP editor fields, in the order the editor and its renderer share.
pub(crate) const BIP_LABELS: [&str; 13] = [
    "Name",
    "Interface",
    "UDP port",
    "Broadcast",
    "Timeout ms",
    "Pacing ms",
    "BBMD",
    "TTL seconds",
    "Renewal sec",
    "Group",
    "Group TTL",
    "Writes",
    "Dangerous",
];

/// BACnet/SC editor fields. Files are paths; their contents are never read here.
pub(crate) const SC_LABELS: [&str; 14] = [
    "Name",
    "Hub URL",
    "Failover URL",
    "CA file",
    "Cert file",
    "Key file",
    "VMAC",
    "Device UUID",
    "Timeout ms",
    "Pacing ms",
    "Connect ms",
    "Reconnect",
    "Writes",
    "Dangerous",
];

/// MS/TP editor fields. The port is one `bacnet mstp-ports` lists.
pub(crate) const MSTP_LABELS: [&str; 10] = [
    "Name",
    "Serial port",
    "MAC (0-127)",
    "Baud",
    "Max master",
    "Info frames",
    "Timeout ms",
    "Pacing ms",
    "Writes",
    "Dangerous",
];

/// Positions of the VMAC and device UUID in [`SC_LABELS`].
pub(crate) const VMAC_FIELD: usize = 6;
/// See [`VMAC_FIELD`].
pub(crate) const UUID_FIELD: usize = 7;

/// State retained while a connection is running or the dialog is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Connection {
    pub(crate) names: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) root: String,
    pub(crate) screen: Screen,
    pub(crate) name: String,
    pub(crate) saved: Option<Profile>,
    /// File checks of the saved profile's resolved settings.
    pub(crate) checks: Checks,
    /// The warnings of `checks` the operator accepted (y); cleared by new checks.
    pub(crate) accepted: Vec<String>,
    /// The transport the editor's fields belong to.
    pub(crate) transport: Transport,
    pub(crate) fields: Vec<String>,
    pub(crate) focus: usize,
    pub(crate) creating: bool,
    pub(crate) error: Option<String>,
    pub(crate) overrides: Overrides,
    /// A queued disk operation disables further mutations until its reply.
    pub(crate) pending: bool,
}

fn optional<T: std::str::FromStr>(text: &str, error: &str) -> Result<Option<T>, String> {
    if text.is_empty() {
        Ok(None)
    } else {
        text.parse().map(Some).map_err(|_| error.to_owned())
    }
}

fn writes_text(writes: Writes) -> String {
    match writes {
        Writes::Never => "never",
        Writes::Session => "session",
    }
    .into()
}

impl Connection {
    pub(crate) fn new(overrides: Overrides) -> Self {
        Self {
            names: Vec::new(),
            selected: 0,
            root: String::new(),
            screen: Screen::List,
            name: String::new(),
            saved: None,
            checks: Checks::default(),
            accepted: Vec::new(),
            transport: Transport::Bip,
            fields: Vec::new(),
            focus: 0,
            creating: false,
            error: None,
            overrides,
            pending: false,
        }
    }
    /// Whether editor field `i` shows an SC identity that is already
    /// assigned, and so can't be edited here (Annex AB.1.5.3).
    pub(crate) fn identity_locked(&self, i: usize) -> bool {
        let Some(sc) = self.saved.as_ref().and_then(|p| p.sc.as_ref()) else {
            return false;
        };
        self.transport == Transport::Sc
            && match i {
                VMAC_FIELD => sc.vmac.is_some(),
                UUID_FIELD => sc.device_uuid.is_some(),
                _ => false,
            }
    }
    /// The editor's field labels.
    pub(crate) fn labels(&self) -> &'static [&'static str] {
        match self.transport {
            Transport::Bip => &BIP_LABELS,
            Transport::Sc => &SC_LABELS,
            Transport::Mstp => &MSTP_LABELS,
        }
    }
    pub(crate) fn reply(&mut self, reply: Result<Reply, String>) {
        self.pending = false;
        self.error = None;
        match reply {
            Ok(Reply::Catalog { names, root }) => {
                self.names = names;
                self.root = root;
                self.selected = self.selected.min(self.names.len().saturating_sub(1));
                self.screen = Screen::List;
            }
            Ok(Reply::Loaded {
                name,
                profile,
                checks,
            }) => {
                if !self.names.contains(&name) {
                    self.names.push(name.clone());
                    self.names.sort();
                }
                self.name = name;
                self.saved = Some(profile);
                self.checks = checks;
                self.accepted.clear();
                self.creating = false;
                self.screen = Screen::Preview;
            }
            Err(error) => self.error = Some(error),
        }
    }
    /// Open the editor on a new B/IP profile.
    pub(crate) fn new_profile(&mut self) {
        self.create(Profile::default());
    }
    fn create(&mut self, profile: Profile) {
        self.name.clear();
        self.saved = None;
        self.checks = Checks::default();
        self.accepted.clear();
        self.creating = true;
        self.edit(profile);
    }
    fn edit(&mut self, p: Profile) {
        self.transport = p.transport;
        let mut fields = vec![self.name.clone()];
        if let Some(b) = &p.bip {
            fields.extend([
                b.interface.to_string(),
                b.port.to_string(),
                b.broadcast.to_string(),
                b.timeout_ms.to_string(),
                b.min_interval_ms.to_string(),
                b.bbmd.map(|v| v.to_string()).unwrap_or_default(),
                b.ttl.to_string(),
                b.renewal_secs.map(|v| v.to_string()).unwrap_or_default(),
                b.multicast.map(|v| v.to_string()).unwrap_or_default(),
                b.multicast_ttl.to_string(),
            ]);
        }
        if let Some(s) = &p.sc {
            fields.extend([
                s.hub_url.clone(),
                s.failover_hub_url.clone().unwrap_or_default(),
                s.ca.display().to_string(),
                s.cert.display().to_string(),
                s.key.display().to_string(),
                s.vmac.map(|v| v.to_string()).unwrap_or_default(),
                s.device_uuid.map(|v| v.to_string()).unwrap_or_default(),
                s.timeout_ms.to_string(),
                s.min_interval_ms.to_string(),
                s.connect_timeout_ms.to_string(),
                if s.reconnect { "on" } else { "off" }.to_string(),
            ]);
        }
        if let Some(m) = &p.mstp {
            // A new profile's MAC starts blank: it is a choice, not a default.
            let mac = if self.creating {
                String::new()
            } else {
                m.mac.to_string()
            };
            fields.extend([
                m.port.clone(),
                mac,
                m.baud.to_string(),
                m.max_master.to_string(),
                m.max_info_frames.to_string(),
                m.timeout_ms.to_string(),
                m.min_interval_ms.to_string(),
            ]);
        }
        fields.push(writes_text(p.safety.writes));
        fields.push(p.safety.dangerous.to_string());
        self.fields = fields;
        self.focus = usize::from(!self.creating);
        self.screen = Screen::Edit;
        self.error = None;
    }
    fn edited(&self) -> Result<Profile, String> {
        super::super::profile::store::validate_name(&self.fields[0])?;
        let mut profile = self.saved.clone().unwrap_or_else(|| match self.transport {
            Transport::Bip => Profile::default(),
            Transport::Sc => Profile::sc_default(),
            Transport::Mstp => Profile::mstp_default(),
        });
        let f = &self.fields;
        if let Some(b) = &mut profile.bip {
            b.interface = f[1]
                .parse()
                .map_err(|_| "Interface must be an IPv4 address")?;
            b.port = f[2].parse().map_err(|_| "UDP port must be 0-65535")?;
            b.broadcast = f[3]
                .parse()
                .map_err(|_| "Broadcast must be an IPv4 address")?;
            b.timeout_ms = f[4]
                .parse()
                .map_err(|_| "Timeout must be an unsigned number")?;
            b.min_interval_ms = f[5]
                .parse()
                .map_err(|_| "Pacing must be an unsigned number")?;
            b.bbmd = if f[6].is_empty() {
                None
            } else {
                Some(crate::foreign::parse_bbmd_arg(&f[6])?)
            };
            b.ttl = f[7].parse().map_err(|_| "TTL must be 0-65535")?;
            b.renewal_secs = optional(&f[8], "Renewal must be an unsigned number")?;
            if b.bbmd.is_none() {
                b.renewal_secs = None;
            }
            // A blank group clears multicast, and the group port with it.
            b.multicast = optional(&f[9], "Group must be an IPv4 multicast address")?;
            if b.multicast.is_none() {
                b.multicast_port = None;
            }
            b.multicast_ttl = f[10]
                .parse()
                .ok()
                .filter(|ttl| *ttl != 0)
                .ok_or("Group TTL must be 1-255")?;
        }
        if let Some(s) = &mut profile.sc {
            s.hub_url = f[1].clone();
            s.failover_hub_url = Some(f[2].clone()).filter(|v| !v.is_empty());
            s.ca = PathBuf::from(&f[3]);
            s.cert = PathBuf::from(&f[4]);
            s.key = PathBuf::from(&f[5]);
            // An assigned identity is read-only here (it came from `saved`);
            // only a new profile's blank or typed values are taken.
            if !self.identity_locked(VMAC_FIELD) {
                s.vmac = optional(&f[6], "VMAC must be 6 hex bytes, not all 0 or all FF")?;
            }
            if !self.identity_locked(UUID_FIELD) {
                s.device_uuid = optional(&f[7], "Device UUID must be a nonzero UUID")?;
            }
            s.timeout_ms = f[8]
                .parse()
                .map_err(|_| "Timeout must be an unsigned number")?;
            s.min_interval_ms = f[9]
                .parse()
                .map_err(|_| "Pacing must be an unsigned number")?;
            s.connect_timeout_ms = f[10]
                .parse()
                .map_err(|_| "Connect ms must be an unsigned number")?;
            s.reconnect = match f[11].as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("Reconnect must be on or off".into()),
            };
        }
        if let Some(m) = &mut profile.mstp {
            m.port = f[1].trim().to_string();
            m.mac = f[2].parse().map_err(|_| "MAC must be 0 to 127")?;
            m.baud = f[3]
                .parse()
                .map_err(|_| "Baud must be 9600, 19200, 38400, 57600, 76800 or 115200")?;
            m.max_master = f[4].parse().map_err(|_| "Max master must be 0 to 127")?;
            m.max_info_frames = f[5].parse().map_err(|_| "Info frames must be 1 to 255")?;
            m.timeout_ms = f[6]
                .parse()
                .map_err(|_| "Timeout must be an unsigned number")?;
            m.min_interval_ms = f[7]
                .parse()
                .map_err(|_| "Pacing must be an unsigned number")?;
        }
        let n = f.len();
        profile.safety.writes = match f[n - 2].as_str() {
            "never" => Writes::Never,
            "session" => Writes::Session,
            _ => return Err("Writes must be never or session".into()),
        };
        profile.safety.dangerous = match f[n - 1].as_str() {
            "false" => false,
            "true" => true,
            _ => return Err("Dangerous must be true or false".into()),
        };
        if self.saved.as_ref().is_some_and(|saved| {
            saved.bip != profile.bip || saved.sc != profile.sc || saved.mstp != profile.mstp
        }) {
            profile.authorized_at_unix = None;
            profile.authorized_safety = None;
        }
        // Raising the write policy needs a fresh authorization too.
        profile.revoke_if_raised();
        profile.validate()?;
        self.overrides.resolve(&profile)?;
        Ok(profile)
    }
    pub(crate) fn resolved(&self) -> Result<Profile, String> {
        self.overrides
            .resolve(self.saved.as_ref().ok_or("No profile selected")?)
    }
    /// Return at most one observable, acknowledged coordinator request.
    pub(crate) fn key(&mut self, key: Key) -> Option<Command> {
        if self.pending {
            return None;
        }
        self.error = None;
        let count = self.fields.len();
        let request = match (&self.screen, key) {
            (Screen::Edit, Key::Next) => {
                self.focus = (self.focus + 1) % count;
                if !self.creating && self.focus == 0 {
                    self.focus = 1;
                }
                None
            }
            (Screen::Edit, Key::Prev) => {
                self.focus = (self.focus + count - 1) % count;
                if !self.creating && self.focus == 0 {
                    self.focus = count - 1;
                }
                None
            }
            (Screen::Edit, Key::Char(_) | Key::Backspace) if self.identity_locked(self.focus) => {
                self.error = Some(
                    "The VMAC and device UUID are kept for good; change them by editing the profile file."
                        .into(),
                );
                None
            }
            (Screen::Edit, Key::Char(ch)) => {
                if self.fields[self.focus].len() < 256 && ch.is_ascii() && !ch.is_control() {
                    self.fields[self.focus].push(ch);
                }
                None
            }
            (Screen::Edit, Key::Backspace) => {
                self.fields[self.focus].pop();
                None
            }
            (Screen::Edit, Key::Discard) => {
                self.fields.clear();
                self.creating = false;
                self.screen = if self.saved.is_some() {
                    Screen::Preview
                } else {
                    Screen::List
                };
                None
            }
            (Screen::Edit, Key::Submit) => match self.edited() {
                Ok(profile) => Some(Request::Save {
                    name: self.fields[0].clone(),
                    profile,
                    create: self.creating,
                    previous: self.saved.clone(),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            (_, Key::New) => {
                self.new_profile();
                None
            }
            (_, Key::NewSc) => {
                self.create(Profile::sc_default());
                None
            }
            (_, Key::NewMstp) => {
                self.create(Profile::mstp_default());
                None
            }
            (Screen::Preview, Key::Edit) => {
                if let Some(profile) = &self.saved {
                    self.edit(profile.clone());
                }
                None
            }
            (_, Key::List) => Some(Request::List),
            (Screen::List, Key::Reload) => Some(Request::List),
            (Screen::List, Key::Next) => {
                self.selected = (self.selected + 1).min(self.names.len().saturating_sub(1));
                None
            }
            (Screen::List, Key::Prev) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            (Screen::List, Key::Submit) => {
                self.names.get(self.selected).cloned().map(Request::Load)
            }
            (Screen::Preview, Key::Reload) => Some(Request::Load(self.name.clone())),
            (Screen::Preview, Key::Confirm) => {
                if self.checks.warnings.is_empty() {
                    self.error = Some("No warnings to accept.".into());
                } else {
                    self.accepted = self.checks.warnings.clone();
                }
                None
            }
            // A warning holds the connection until accepted (#968). The
            // coordinator checks again against fresh checks.
            (Screen::Preview, Key::Submit | Key::Connect | Key::Attest)
                if self.checks.errors.is_empty() && self.accepted != self.checks.warnings =>
            {
                self.error =
                    Some("Review the warnings above: y accepts them for this connection.".into());
                None
            }
            // A failed file check is refused again (freshly) by the
            // coordinator, so the request goes out and names the path.
            (Screen::Preview, key @ (Key::Submit | Key::Connect | Key::Attest)) => {
                match self.resolved() {
                    Err(error) => {
                        self.error = Some(error);
                        None
                    }
                    Ok(profile) if profile.authorized_at_unix.is_none() && key != Key::Attest => {
                        self.error = Some(
                            "Press a to attest authorization and connect, or Esc to cancel".into(),
                        );
                        None
                    }
                    Ok(_) => Some(Request::Connect {
                        name: self.name.clone(),
                        profile: self.saved.clone().expect("resolved profile"),
                        attest: key == Key::Attest,
                        accepted: self.accepted.clone(),
                    }),
                }
            }
            _ => None,
        };
        request.map(|request| {
            self.pending = true;
            Command::Profile(request)
        })
    }
}
