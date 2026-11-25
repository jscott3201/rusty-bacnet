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
//! is, or, with `std`, owned text where it names a value only known at
//! run time, such as a host name or another error's message. A build
//! without `std`, firmware's, holds only the label form, so a refusal there
//! owns nothing and an [`Error`](super::Error) gains no drop code from it.

use core::fmt;

use super::decode::{placeholder_count, write_label};

/// The lifecycle state an [`Error::State`](super::Error::State) refusal
/// found its component in: a state of the component itself, which it
/// decided from its own bookkeeping, before any I/O.
///
/// Two neighbours are not lifecycle states. A resource one holder takes at
/// a time (a lock, a lease, a seat) that another holder has is a cap of one
/// reached, [`Error::LimitReached`](super::Error::LimitReached): the call
/// may succeed once the holder lets go. And an I/O failure the operating
/// system or a socket reported is `Error::Transport`.
/// A stopped transport's send still fails with
/// `Error::Transport` of kind `NotConnected`, as
/// before this type existed: the client and the bindings classify that
/// error as a link that is down, so moving it to [`Lifecycle::NotRunning`]
/// is a change of behaviour #1833 makes on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lifecycle {
    /// The component has already started, so a second start is refused, as
    /// is a call that only a component not yet started takes, such as
    /// handing it a lease to hold from its start.
    AlreadyStarted,
    /// The component is not running: it has not started yet, or it has
    /// stopped. The call needs it running.
    NotRunning,
    /// A stop has begun and not finished; the call is refused while it
    /// completes.
    Stopping,
    /// Running, but its session with the peer it works through, such as a
    /// BACnet/SC node's hub, is not up.
    NotConnected,
    /// Closed for good: stopped in a way it can't start from again, or the
    /// channel to it closed under the call.
    Closed,
}

/// The text of a local refusal (#1833), for a person to read: a static
/// label whose placeholders take up to two numbers, built when displayed,
/// or, with `std`, owned text. The text is not API: match the
/// [`Error`](super::Error) variant, and read a fact a caller acts on from a
/// typed variant or field, not from here.
///
/// A label's placeholders are those [`RangeError`](super::RangeError)'s
/// take, such as `{}` for a number in decimal and `{:#x}` in hexadecimal.
/// Each constructor takes as many numbers as its label has placeholders,
/// which a debug build checks.
///
/// ```
/// use bacnet_types::error::{Error, Refusal};
///
/// let refused =
///     Error::InvalidArgument(Refusal::with_value("MS/TP MAC must be 1 byte, got {}", 2));
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
    /// form.
    #[cfg(feature = "std")]
    Owned(Box<str>),
}

impl Refusal {
    /// A refusal described by `label`, which has no placeholder.
    pub const fn new(label: &'static str) -> Self {
        debug_assert!(placeholder_count(label) == 0, "a label with placeholders");
        Self(Text::Label {
            label,
            values: [0; 2],
        })
    }

    /// A refusal described by `label`, whose one placeholder stands for
    /// `value`.
    pub const fn with_value(label: &'static str, value: u64) -> Self {
        debug_assert!(
            placeholder_count(label) == 1,
            "a label without one placeholder"
        );
        Self(Text::Label {
            label,
            values: [value, 0],
        })
    }

    /// A refusal described by `label`, whose two placeholders stand for
    /// `first` and `second`.
    pub const fn with_values(label: &'static str, first: u64, second: u64) -> Self {
        debug_assert!(
            placeholder_count(label) == 2,
            "a label without two placeholders"
        );
        Self(Text::Label {
            label,
            values: [first, second],
        })
    }

    /// A refusal described by `text`, for a message that names a value
    /// only known at run time. Only with `std`.
    #[cfg(feature = "std")]
    pub fn text(text: impl Into<String>) -> Self {
        Self(Text::Owned(text.into().into_boxed_str()))
    }

    /// The same refusal, its text led by `context` and `: `. Only with
    /// `std`.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn with_context(self, context: &str) -> Self {
        Self::text(format!("{context}: {self}"))
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Text::Label { label, values } => write_label(f, label, *values),
            #[cfg(feature = "std")]
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
        let label = Refusal::with_values("{} of {} slots", 3, 4);
        for (error, text) in [
            (
                Error::State(Lifecycle::AlreadyStarted, Refusal::new("started")),
                "started",
            ),
            (Error::InvalidConfig(label.clone()), "3 of 4 slots"),
            (Error::InvalidArgument(label.clone()), "3 of 4 slots"),
            (
                Error::Unsupported(Refusal::with_value("no link {:#x}", 0x1F)),
                "no link 0x1f",
            ),
            (Error::LimitReached(label), "3 of 4 slots"),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    /// A label given fewer numbers than it has placeholders would print 0
    /// for the missing one; a debug build refuses it at construction.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "a label without two placeholders")]
    fn a_label_given_too_few_numbers_is_refused() {
        let _ = Refusal::with_values("only {}", 1, 2);
    }

    /// The same for a label with placeholders given no number.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "a label with placeholders")]
    fn a_label_given_no_numbers_is_refused() {
        let _ = Refusal::new("{} of {}");
    }

    /// Context leads the text and keeps the variant; an error that is not a
    /// local refusal is returned as it is.
    #[cfg(feature = "std")]
    #[test]
    fn context_keeps_the_variant() {
        let full = Error::LimitReached(Refusal::with_value("{} rows", 129));
        match full.with_context("BDT configuration error") {
            Error::LimitReached(refusal) => {
                assert_eq!(refusal.to_string(), "BDT configuration error: 129 rows");
            }
            other => panic!("{other:?}"),
        }
        let owned = Error::State(Lifecycle::Stopping, Refusal::text("host 'x'"));
        assert!(matches!(
            owned.with_context("stop"),
            Error::State(Lifecycle::Stopping, ref text) if text.to_string() == "stop: host 'x'"
        ));
        assert!(matches!(
            Error::buffer_too_short(2, 1).with_context("ignored"),
            Error::BufferTooShort { need: 2, have: 1 }
        ));
    }
}
