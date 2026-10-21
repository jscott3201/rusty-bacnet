//! The Capture screen's filter: space-separated terms, all of which a
//! frame must match.
//!
//! - `dev:N`: frames from or to device N's address in the device table (a
//!   routed device's as `NET:0xMAC`), and N's I-Am. Addresses are looked up
//!   again as the device table learns them.
//! - `mac:TEXT`: a source or destination address containing TEXT.
//! - `net:N`: an NPDU with SNET or DNET N.
//! - `svc:NAME`: that service's requests, ACKs and errors. Case, `-` and
//!   `_` don't matter: `svc:ReadProperty` is `READ_PROPERTY`.
//! - `pdu:TYPE`: an APDU type by the start or end of its name
//!   (`pdu:confirmed`, `pdu:ack`, `pdu:error`), `pdu:network` for network
//!   messages, `pdu:link` for frames without an NPDU.
//! - `errors`: an Error, Reject or Abort, a NAK, a frame that doesn't
//!   decode, a line error.
//! - `bcast`: link and network broadcasts.
//! - anything else: text anywhere in the row.

use super::CaptureFrame;

/// One term.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    /// A device instance, and its addresses as the device table has them:
    /// its own, or for a routed device its routed station, `2:0x05`.
    Device(u32, Vec<String>),
    Mac(String),
    Net(u16),
    Service(String),
    Pdu(String),
    Errors,
    Broadcasts,
    Text(String),
}

/// A parsed filter; the empty one keeps every frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    terms: Vec<Term>,
}

/// Lowercase, letters and digits only.
fn squash(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl Filter {
    /// Parse the filter line.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let number = |key: &str, value: &str| {
            value
                .parse::<u32>()
                .map_err(|_| format!("'{key}:{value}': {key} takes a number"))
        };
        let mut terms = Vec::new();
        for word in text.split_whitespace() {
            let term = match word.split_once(':') {
                Some(("dev" | "device", value)) => Term::Device(number("dev", value)?, Vec::new()),
                Some(("mac", value)) if !value.is_empty() => Term::Mac(value.to_lowercase()),
                Some(("net", value)) => Term::Net(
                    u16::try_from(number("net", value)?)
                        .map_err(|_| format!("'net:{value}': a network is 0 to 65535"))?,
                ),
                Some(("svc" | "service", value)) if !squash(value).is_empty() => {
                    Term::Service(squash(value))
                }
                Some(("pdu", value)) if !squash(value).is_empty() => Term::Pdu(squash(value)),
                _ if matches!(word, "errors" | "error" | "err") => Term::Errors,
                _ if matches!(word, "bcast" | "broadcast" | "broadcasts") => Term::Broadcasts,
                _ => Term::Text(word.to_lowercase()),
            };
            terms.push(term);
        }
        Ok(Self { terms })
    }

    /// Look up the addresses of each `dev:` term's device; true when one
    /// changed, so the ring must be filtered again.
    pub(crate) fn resolve(&mut self, addresses: impl Fn(u32) -> Vec<String>) -> bool {
        let mut changed = false;
        for term in &mut self.terms {
            if let Term::Device(instance, found) = term {
                let now = addresses(*instance);
                changed |= *found != now;
                *found = now;
            }
        }
        changed
    }

    /// Whether a `dev:` term waits on the device table.
    pub(crate) fn has_devices(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, Term::Device(..)))
    }

    /// Whether `frame` passes every term.
    pub(crate) fn matches(&self, frame: &CaptureFrame) -> bool {
        self.terms.iter().all(|term| term_matches(term, frame))
    }
}

fn term_matches(term: &Term, frame: &CaptureFrame) -> bool {
    let summary = &frame.summary;
    match term {
        Term::Device(instance, addresses) => {
            summary.device == Some(*instance)
                || addresses
                    .iter()
                    .any(|a| frame.source == *a || frame.destination == *a)
        }
        Term::Mac(text) => {
            frame.source.to_lowercase().contains(text)
                || frame.destination.to_lowercase().contains(text)
        }
        Term::Net(net) => summary.snet == Some(*net) || summary.dnet == Some(*net),
        Term::Service(name) => summary
            .service
            .as_deref()
            .is_some_and(|s| squash(s) == *name),
        Term::Pdu(kind) => match (&summary.pdu, &summary.network_message) {
            (Some(pdu), _) => {
                let pdu = squash(pdu);
                pdu.starts_with(kind.as_str()) || pdu.ends_with(kind.as_str())
            }
            (None, Some(_)) => kind == "network",
            (None, None) => kind == "link",
        },
        Term::Errors => summary.problem.is_some(),
        Term::Broadcasts => summary.broadcast,
        Term::Text(text) => row_text(frame).contains(text.as_str()),
    }
}

/// Everything a row shows, lowercased, for free text.
fn row_text(frame: &CaptureFrame) -> String {
    let summary = &frame.summary;
    let mut text = format!(
        "{} {} {} {} {}",
        frame.framing.label(),
        frame.source,
        frame.destination,
        summary.link,
        frame.direction.label()
    );
    if let Some(via) = &frame.via {
        text.push_str(" via ");
        text.push_str(via);
    }
    for part in [
        &summary.pdu,
        &summary.service,
        &summary.problem,
        &summary.network_message,
    ]
    .into_iter()
    .flatten()
    {
        text.push(' ');
        text.push_str(part);
    }
    if let Some(id) = summary.invoke_id {
        text.push_str(&format!(" #{id}"));
    }
    text.to_lowercase()
}
