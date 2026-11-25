//! Local refusals (#1833): a call the stack turned down on its own side,
//! before or without any exchange with a peer, because of a lifecycle
//! state, a configuration value, an argument, what the platform or build
//! offers, or a resource cap. The variant of [`Error`](super::Error) says
//! which, so a caller matches on it without reading text:
//! [`Error::State`](super::Error::State) with its [`Lifecycle`],
//! [`Error::InvalidConfig`](super::Error::InvalidConfig),
//! [`Error::InvalidArgument`](super::Error::InvalidArgument),
//! [`Error::Unsupported`](super::Error::Unsupported) and
//! [`Error::LimitReached`](super::Error::LimitReached).
//!
//! Each carries a [`Refusal`], the text for a person: a static label with
//! up to two numbers, built when displayed as [`RangeError`](super::RangeError)'s
//! is, or, with `alloc`, owned text where it names a value only known at
//! run time, such as a host name or another error's message.

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::{boxed::Box, string::String};
use core::fmt;

use super::decode::write_label;

/// The lifecycle state an [`Error::State`](super::Error::State) refusal
/// found its component in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lifecycle {
    /// Started already: a second start, or a call that needs a component
    /// that has not started yet.
    AlreadyStarted,
    /// Not running: not started yet, or stopped, for a call that needs it
    /// running.
    NotRunning,
    /// Running, but not connected to the peer it works through, such as a
    /// BACnet/SC node's hub.
    NotConnected,
    /// Closed for good: stopped in a way it can't start from again, or the
    /// channel to it closed under the call.
    Closed,
}

/// The text of a local refusal (#1833), for a person to read: a static
/// label whose placeholders take up to two numbers, built when displayed,
/// or, with `alloc`, owned text.
///
/// A label's placeholders are those [`RangeError`](super::RangeError)'s
/// take, such as `{}` for a number in decimal and `{:#x}` in hexadecimal.
///
/// ```
/// use bacnet_types::error::{Error, Refusal};
///
/// let refused = Error::InvalidArgument(
///     Refusal::new("MS/TP MAC must be 1 byte, got {}").with_value(2),
/// );
/// assert_eq!(refused.to_string(), "MS/TP MAC must be 1 byte, got 2");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Refusal(Text);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Text {
    Label {
        label: &'static str,
        values: [u64; 2],
    },
    /// Boxed, not a `String`, so the refusal is no wider than its label
    /// form and fits beside the decoder's error in [`Error`](super::Error).
    #[cfg(feature = "alloc")]
    Owned(Box<str>),
}

impl Refusal {
    /// A refusal described by `label`.
    pub const fn new(label: &'static str) -> Self {
        Self(Text::Label {
            label,
            values: [0; 2],
        })
    }

    /// A refusal described by `text`, for a message that names a value
    /// only known at run time. Only with `alloc`.
    #[cfg(feature = "alloc")]
    pub fn text(text: impl Into<String>) -> Self {
        Self(Text::Owned(text.into().into_boxed_str()))
    }

    /// The same refusal, with the number the label's one placeholder stands
    /// for. Owned text has no placeholders and stays as it is.
    #[must_use]
    pub fn with_value(self, value: u64) -> Self {
        self.with_values(value, 0)
    }

    /// The same refusal, with the numbers the label's two placeholders
    /// stand for. Owned text has no placeholders and stays as it is.
    #[must_use]
    pub fn with_values(self, first: u64, second: u64) -> Self {
        match self.0 {
            Text::Label { label, .. } => Self(Text::Label {
                label,
                values: [first, second],
            }),
            #[cfg(feature = "alloc")]
            owned @ Text::Owned(_) => Self(owned),
        }
    }

    /// The static label, placeholders unfilled; `None` for owned text.
    pub fn label(&self) -> Option<&'static str> {
        match self.0 {
            Text::Label { label, .. } => Some(label),
            #[cfg(feature = "alloc")]
            Text::Owned(_) => None,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Text::Label { label, values } => write_label(f, label, *values),
            #[cfg(feature = "alloc")]
            Text::Owned(text) => f.write_str(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Error;
    use super::*;

    /// Each local refusal displays as its text alone, with no category in
    /// front: the variant says what kind of refusal it is.
    #[test]
    fn refusals_display_their_text_alone() {
        let label = Refusal::new("{} of {} slots").with_values(3, 4);
        for (error, text) in [
            (
                Error::State(Lifecycle::AlreadyStarted, Refusal::new("started")),
                "started",
            ),
            (Error::InvalidConfig(label.clone()), "3 of 4 slots"),
            (Error::InvalidArgument(label.clone()), "3 of 4 slots"),
            (
                Error::Unsupported(Refusal::new("no such link")),
                "no such link",
            ),
            (Error::LimitReached(label), "3 of 4 slots"),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    /// A label keeps its placeholders unfilled; owned text has none, and
    /// the number builders leave it as it is.
    #[cfg(feature = "alloc")]
    #[test]
    fn owned_text_has_no_label_and_takes_no_numbers() {
        let label = Refusal::new("got {}").with_value(7);
        assert_eq!(label.label(), Some("got {}"));
        assert_eq!(label.to_string(), "got 7");
        let owned = Refusal::text("host 'x' is unknown").with_values(1, 2);
        assert_eq!(owned.label(), None);
        assert_eq!(owned.to_string(), "host 'x' is unknown");
    }
}
