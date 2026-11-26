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
///
/// A transport reports `AlreadyStarted` for a second start, `NotRunning`
/// for a send before start or, when it can start again (B/IP, B/IPv6,
/// Ethernet), after a stop, `Closed` for a send after the stop of one that
/// can't (a loopback, BACnet/SC, MS/TP), and `NotConnected` while its hub
/// or BBMD isn't known. Nothing of such a send was attempted. The network
/// layer and the router's application port report `AlreadyStarted`, and
/// the router's application port, its replies and the network layer's
/// sealed responses `Closed`.
/// The endpoint's session and ingress report all but `NotConnected`,
/// deciding between `NotRunning`, `Stopping` and `Closed` from their own
/// state. The server reports `Closed` once its stop has begun or a cleanup
/// task failed, and the client `Stopping` while its seat's node stops and
/// `Closed` once its seat or its coordinator has closed. A request that
/// went out before the client or its link stopped under it, or the client's
/// stop cut off while it waited, is not a lifecycle state but its own
/// outcome, [`Error::RequestCutOff`](super::Error::RequestCutOff): a state
/// refusal promises nothing of the request went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Lifecycle {
    /// The component has started already. A second start is refused, and so
    /// is a call that only a component not yet started takes, such as
    /// handing it a lease to hold from its start.
    AlreadyStarted,
    /// The component is not running, but it can be: it has not started yet,
    /// or it stopped in a way that lets it start again.
    NotRunning,
    /// A stop has begun and not finished; the call is refused while it
    /// completes. Also a transport an abort ended, whose start is refused
    /// until a stop has joined what the abort left.
    Stopping,
    /// Running, but its session with the peer it works through, such as a
    /// BACnet/SC node's hub, is not up.
    NotConnected,
    /// Closed for good: it can't be started again, or the channel to it
    /// closed under the call.
    Closed,
}

impl Lifecycle {
    /// The state's name in lowercase snake case, which stays the same from
    /// release to release (#1833): `already_started`, `not_running`,
    /// `stopping`, `not_connected` or `closed`. The Python bindings give it
    /// as `BacnetStateError.state`.
    ///
    /// ```
    /// use bacnet_types::error::Lifecycle;
    ///
    /// assert_eq!(Lifecycle::NotRunning.name(), "not_running");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AlreadyStarted => "already_started",
            Self::NotRunning => "not_running",
            Self::Stopping => "stopping",
            Self::NotConnected => "not_connected",
            Self::Closed => "closed",
        }
    }
}

/// The text of a local refusal (#1833), for a person to read: a static
/// label whose placeholders take up to two numbers, built when displayed,
/// or, with `std`, owned text. The text is not API: match the
/// [`Error`](super::Error) variant, and read a fact a caller acts on from a
/// typed variant or field, not from here.
///
/// A label's placeholders are those [`RangeError`](super::RangeError)'s
/// take, such as `{}` for a number in decimal and `{:#x}` in hexadecimal,
/// and the numbers come as they do for it: [`new`](Self::new), then
/// [`with_value`](Self::with_value) or [`with_values`](Self::with_values)
/// for a label with one or two placeholders. A debug build checks the count
/// at that last step, and when it displays a label given no numbers. A
/// release build doesn't check: a placeholder with no number prints 0, and a
/// number with no placeholder is not printed.
///
/// ```
/// use bacnet_types::error::{Error, Refusal};
///
/// let refused =
///     Error::InvalidArgument(Refusal::new("MS/TP MAC must be 1 byte, got {}").with_value(2));
/// assert_eq!(refused.to_string(), "MS/TP MAC must be 1 byte, got 2");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Refusal(Text);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Text {
    /// A label as [`Refusal::new`] took it, given no numbers.
    Label(&'static str),
    /// A label with the numbers its placeholders stand for.
    Numbered {
        label: &'static str,
        values: [u64; 2],
    },
    /// Boxed, not a `String`, so the refusal is no wider than its label
    /// forms.
    #[cfg(feature = "std")]
    Owned(Box<str>),
}

impl Refusal {
    /// A refusal described by `label`; a label with placeholders takes its
    /// numbers from [`with_value`](Self::with_value) or
    /// [`with_values`](Self::with_values).
    pub const fn new(label: &'static str) -> Self {
        Self(Text::Label(label))
    }

    /// The same refusal, with the number the label's one placeholder stands
    /// for.
    #[must_use]
    pub fn with_value(self, value: u64) -> Self {
        self.numbered(1, [value, 0])
    }

    /// The same refusal, with the numbers the label's two placeholders
    /// stand for.
    #[must_use]
    pub fn with_values(self, first: u64, second: u64) -> Self {
        self.numbered(2, [first, second])
    }

    /// `values` for a label of `count` placeholders, given once; owned text
    /// takes none and stays as it is.
    fn numbered(self, count: usize, values: [u64; 2]) -> Self {
        debug_assert!(
            matches!(self.0, Text::Label(label) if placeholder_count(label) == count),
            "a refusal's numbers must match its label's placeholders, given once"
        );
        match self.0 {
            Text::Label(label) | Text::Numbered { label, .. } => {
                Self(Text::Numbered { label, values })
            }
            #[cfg(feature = "std")]
            Text::Owned(_) => self,
        }
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
            Text::Label(label) => {
                debug_assert!(
                    placeholder_count(label) == 0,
                    "a label with placeholders given no numbers"
                );
                write_label(f, label, [0; 2])
            }
            Text::Numbered { label, values } => write_label(f, label, *values),
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
        let label = Refusal::new("{} of {} slots").with_values(3, 4);
        for (error, text) in [
            (
                Error::State(Lifecycle::AlreadyStarted, Refusal::new("started")),
                "started",
            ),
            (Error::InvalidConfig(label.clone()), "3 of 4 slots"),
            (Error::InvalidArgument(label.clone()), "3 of 4 slots"),
            (
                Error::Unsupported(Refusal::new("no link {:#x}").with_value(0x1F)),
                "no link 0x1f",
            ),
            (Error::LimitReached(label), "3 of 4 slots"),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    /// Each state's name is the one the Python bindings expose, which a
    /// script may compare: pinned (#1833).
    #[test]
    fn each_state_keeps_its_name() {
        for (state, name) in [
            (Lifecycle::AlreadyStarted, "already_started"),
            (Lifecycle::NotRunning, "not_running"),
            (Lifecycle::Stopping, "stopping"),
            (Lifecycle::NotConnected, "not_connected"),
            (Lifecycle::Closed, "closed"),
        ] {
            assert_eq!(state.name(), name);
        }
    }

    /// A label given more numbers than it has placeholders would drop one,
    /// and one given fewer would print 0; a debug build refuses either when
    /// they are given.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "must match its label's placeholders")]
    fn a_label_given_too_many_numbers_is_refused() {
        let _ = Refusal::new("only {}").with_values(1, 2);
    }

    /// The same for numbers given twice.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "must match its label's placeholders")]
    fn numbers_given_twice_are_refused() {
        let _ = Refusal::new("{}").with_value(1).with_value(2);
    }

    /// A label with placeholders given no numbers, which a debug build
    /// catches when it displays it.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "a label with placeholders given no numbers")]
    fn a_label_given_no_numbers_is_refused() {
        let _ = Refusal::new("{} of {}").to_string();
    }

    /// A release build prints 0 for a placeholder with no number.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_prints_zero_for_a_missing_number() {
        assert_eq!(Refusal::new("{} of {}").to_string(), "0 of 0");
        assert_eq!(Refusal::new("{} of {}").with_value(3).to_string(), "3 of 0");
    }

    /// Context leads the text and keeps the variant; an error that is not a
    /// local refusal is returned as it is.
    #[cfg(feature = "std")]
    #[test]
    fn context_keeps_the_variant() {
        let full = Error::LimitReached(Refusal::new("{} rows").with_value(129));
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
