//! Persisted state that can't be loaded or saved (#1833, #2047):
//! [`Error::Persistence`](super::Error::Persistence) and the
//! [`PersistenceError`] it carries, which every snapshot codec of
//! `bacnet-objects` returns and its storage backends report.

use core::fmt;

/// What kind of trouble a [`PersistenceError`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PersistenceErrorKind {
    /// The octets end before the format says they do: a save cut short.
    Truncated,
    /// The octets don't start with the format's magic tag.
    NotThisFormat,
    /// A sound snapshot of a schema version this build doesn't read.
    UnknownVersion(u16),
    /// A body kind this build doesn't read.
    UnknownKind(u8),
    /// A sound snapshot saved for another object, or for another object type
    /// than the one asked for.
    OtherObject,
    /// More octets than the format's size cap: not something a save wrote.
    TooLarge,
    /// Damage: a bad checksum or length, or a value no save writes.
    Damaged,
    /// Refused as it was encoded: the snapshot holds a value, or reaches a
    /// size, the format can't keep. Nothing was written.
    RefusedOnEncode,
    /// The storage behind the snapshot failed with no operating-system
    /// error to report (#1833): a save that failed, or panicked, or whose
    /// failure was already reported to an earlier caller, or a stored copy
    /// that changed while it was read. Whatever is stored may still be
    /// sound. An I/O failure the operating system reported is
    /// `Error::Transport` instead.
    BackendFailed,
}

/// Why persisted state didn't encode, decode, load or save.
///
/// Its own text is static, so a firmware image reports it without
/// allocating; formatting it ([`Display`](fmt::Display)) is the only cost,
/// and only when the application asks. The text names the format, then the
/// field at fault when there is one, then the reason. With `std` a caller
/// may put context in front, such as a file's path
/// (`Error::with_context`); the kind stays.
///
/// A storage backend of the application's own reports its failure with
/// [`new`](Self::new), of kind
/// [`BackendFailed`](PersistenceErrorKind::BackendFailed), or
/// [`Damaged`](PersistenceErrorKind::Damaged) for octets no save wrote.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PersistenceError(Inner);

/// The static form: what every build carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Plain {
    kind: PersistenceErrorKind,
    format: &'static str,
    field: Option<&'static str>,
    detail: &'static str,
}

/// Without `std` the error owns nothing.
#[cfg(not(feature = "std"))]
type Inner = Plain;

/// With `std`, the static form or, boxed so the error stays no wider, the
/// static form with context in front of its text.
#[cfg(feature = "std")]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Inner {
    Plain(Plain),
    Located(Box<(Plain, Box<str>)>),
}

impl PersistenceError {
    /// An error of `kind`, described by the static `detail`.
    pub const fn new(kind: PersistenceErrorKind, detail: &'static str) -> Self {
        Self(Inner::from_plain(Plain {
            kind,
            format: "",
            field: None,
            detail,
        }))
    }

    /// The same error, about the field `field`.
    #[must_use]
    pub fn in_field(mut self, field: &'static str) -> Self {
        self.plain_mut().field = Some(field);
        self
    }

    /// The same error, its text led by `format` (such as "log snapshot").
    #[must_use]
    pub fn in_format(mut self, format: &'static str) -> Self {
        self.plain_mut().format = format;
        self
    }

    /// The same error with another kind: a check shared by a codec's encode
    /// and decode reports [`RefusedOnEncode`](PersistenceErrorKind::RefusedOnEncode)
    /// on one and damage on the other.
    #[must_use]
    pub fn with_kind(mut self, kind: PersistenceErrorKind) -> Self {
        self.plain_mut().kind = kind;
        self
    }

    /// What kind of trouble this is.
    pub fn kind(&self) -> PersistenceErrorKind {
        self.plain().kind
    }

    /// Whether the octets are damaged, so a backend that keeps two copies
    /// (as the log and Audit Log file backends keep two slots) should pass
    /// over this one for the other. `false` for a sound snapshot this build
    /// must not take ([`UnknownVersion`](PersistenceErrorKind::UnknownVersion),
    /// [`UnknownKind`](PersistenceErrorKind::UnknownKind),
    /// [`OtherObject`](PersistenceErrorKind::OtherObject)), where loading
    /// should stop rather than fall back to an older copy, for every
    /// encode refusal
    /// ([`RefusedOnEncode`](PersistenceErrorKind::RefusedOnEncode)), and for
    /// a storage failure
    /// ([`BackendFailed`](PersistenceErrorKind::BackendFailed)), which says
    /// nothing of the octets.
    pub fn is_damage(&self) -> bool {
        matches!(
            self.kind(),
            PersistenceErrorKind::Truncated
                | PersistenceErrorKind::NotThisFormat
                | PersistenceErrorKind::TooLarge
                | PersistenceErrorKind::Damaged
        )
    }

    /// The same error with `context` and `: ` in front of its text, which
    /// `Error::with_context` adds. Only with
    /// `std`.
    #[cfg(feature = "std")]
    pub(super) fn with_context(self, context: &str) -> Self {
        let (plain, text) = match self.0 {
            Inner::Plain(plain) => (plain, Box::from(context)),
            Inner::Located(located) => {
                let (plain, inner) = *located;
                (plain, format!("{context}: {inner}").into_boxed_str())
            }
        };
        Self(Inner::Located(Box::new((plain, text))))
    }

    fn plain(&self) -> &Plain {
        self.0.plain()
    }

    fn plain_mut(&mut self) -> &mut Plain {
        self.0.plain_mut()
    }
}

/// Without `std` the inner form is the static one.
#[cfg(not(feature = "std"))]
impl Plain {
    const fn from_plain(plain: Plain) -> Self {
        plain
    }

    fn plain(&self) -> &Plain {
        self
    }

    fn plain_mut(&mut self) -> &mut Plain {
        self
    }
}

/// With `std`, the static form wherever it is.
#[cfg(feature = "std")]
impl Inner {
    const fn from_plain(plain: Plain) -> Self {
        Inner::Plain(plain)
    }

    fn plain(&self) -> &Plain {
        match self {
            Inner::Plain(plain) => plain,
            Inner::Located(located) => &located.0,
        }
    }

    fn plain_mut(&mut self) -> &mut Plain {
        match self {
            Inner::Plain(plain) => plain,
            Inner::Located(located) => &mut located.0,
        }
    }
}

impl fmt::Display for Plain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.format.is_empty() {
            write!(f, "{} ", self.format)?;
        }
        if let Some(field) = self.field {
            write!(f, "{field} ")?;
        }
        match self.kind {
            PersistenceErrorKind::UnknownVersion(version) => {
                write!(f, "schema version {version} is unsupported")
            }
            PersistenceErrorKind::UnknownKind(kind) => write!(f, "{} ({kind})", self.detail),
            _ => f.write_str(self.detail),
        }
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "std")]
        if let Inner::Located(located) = &self.0 {
            write!(f, "{}: ", located.1)?;
        }
        fmt::Display::fmt(self.plain(), f)
    }
}

impl core::error::Error for PersistenceError {}

#[cfg(test)]
mod tests {
    use super::super::Error;
    use super::*;

    /// A persisted-state error prints the snapshot's text alone, with no
    /// category in front, and keeps its kind (#1833).
    #[test]
    fn persistence_errors_print_the_snapshot_text_and_keep_the_kind() {
        let damaged = PersistenceError::new(PersistenceErrorKind::Damaged, "checksum mismatch")
            .in_field("rows")
            .in_format("log snapshot");
        let error = Error::from(damaged);
        assert_eq!(error.to_string(), "log snapshot rows checksum mismatch");
        assert!(matches!(&error, Error::Persistence(e) if e.is_damage()));
        let backend = Error::from(PersistenceError::new(
            PersistenceErrorKind::BackendFailed,
            "storage unavailable",
        ));
        assert_eq!(backend.to_string(), "storage unavailable");
        assert!(matches!(
            &backend,
            Error::Persistence(e)
                if e.kind() == PersistenceErrorKind::BackendFailed && !e.is_damage()
        ));
        let version = PersistenceError::new(PersistenceErrorKind::UnknownVersion(9), "");
        assert_eq!(
            Error::from(version.in_format("Tags file")).to_string(),
            "Tags file schema version 9 is unsupported"
        );
        let refused = PersistenceError::new(PersistenceErrorKind::Damaged, "cap")
            .with_kind(PersistenceErrorKind::RefusedOnEncode);
        assert_eq!(refused.kind(), PersistenceErrorKind::RefusedOnEncode);
    }

    /// Which kinds are damage, so a two-slot backend passes over the copy
    /// for the other one: pinned, as the fallback acts on it.
    #[test]
    fn only_damage_kinds_are_damage() {
        for (kind, damage) in [
            (PersistenceErrorKind::Truncated, true),
            (PersistenceErrorKind::NotThisFormat, true),
            (PersistenceErrorKind::TooLarge, true),
            (PersistenceErrorKind::Damaged, true),
            (PersistenceErrorKind::UnknownVersion(2), false),
            (PersistenceErrorKind::UnknownKind(7), false),
            (PersistenceErrorKind::OtherObject, false),
            (PersistenceErrorKind::RefusedOnEncode, false),
            (PersistenceErrorKind::BackendFailed, false),
        ] {
            assert_eq!(
                PersistenceError::new(kind, "x").is_damage(),
                damage,
                "{kind:?}"
            );
        }
    }

    /// A persisted-state error takes no Reject reason: a responder answers
    /// it as it answered the text it replaces.
    #[test]
    fn persistence_errors_take_no_reject_reason() {
        let error = Error::from(PersistenceError::new(PersistenceErrorKind::Damaged, "x"));
        assert_eq!(error.reject_reason(), None);
        assert!(matches!(error.into_request_reject(), Error::Persistence(_)));
    }

    /// Context goes in front of the text, more context in front of that,
    /// and the kind stays: a file backend names its path this way (#2730).
    #[cfg(feature = "std")]
    #[test]
    fn context_leads_the_text_and_keeps_the_kind() {
        let error = Error::from(
            PersistenceError::new(PersistenceErrorKind::NotThisFormat, "has no valid header")
                .in_format("Tags file"),
        )
        .with_context("/var/lib/bacnet/device.tags");
        assert_eq!(
            error.to_string(),
            "/var/lib/bacnet/device.tags: Tags file has no valid header"
        );
        let Error::Persistence(refused) = error.with_context("Device 1") else {
            panic!("the variant stays");
        };
        assert!(refused.is_damage());
        assert_eq!(
            refused.to_string(),
            "Device 1: /var/lib/bacnet/device.tags: Tags file has no valid header"
        );
        // The codec's own setters still reach the static form.
        let renamed = refused.in_field("tags");
        assert_eq!(
            renamed.to_string(),
            "Device 1: /var/lib/bacnet/device.tags: Tags file tags has no valid header"
        );
    }
}
