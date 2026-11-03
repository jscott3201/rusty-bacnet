//! The one error every snapshot codec returns.

use core::fmt;

use bacnet_types::error::Error;

/// What kind of trouble a [`SnapshotError`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotErrorKind {
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
    OutOfRange,
}

/// Why a snapshot didn't encode or decode.
///
/// It carries only static text, so a firmware image reports it without
/// allocating; formatting it ([`Display`](fmt::Display)) is the only cost,
/// and only when the application asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotError {
    kind: SnapshotErrorKind,
    field: Option<&'static str>,
    detail: &'static str,
}

impl SnapshotError {
    /// An error of `kind`, described by `detail`.
    pub(crate) const fn new(kind: SnapshotErrorKind, detail: &'static str) -> Self {
        Self {
            kind,
            field: None,
            detail,
        }
    }

    /// Damage, described by `detail`.
    pub(crate) const fn damaged(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::Damaged, detail)
    }

    /// The octets end early.
    pub(crate) const fn truncated(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::Truncated, detail)
    }

    /// A value the format can't keep, refused on encode.
    pub(crate) const fn out_of_range(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::OutOfRange, detail)
    }

    /// The same error, about the field `field`.
    pub(crate) const fn in_field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }

    /// What kind of trouble this is.
    pub const fn kind(&self) -> SnapshotErrorKind {
        self.kind
    }

    /// The part of the snapshot at fault, when the codec names one.
    pub const fn field(&self) -> Option<&'static str> {
        self.field
    }

    /// Whether the octets are damaged, so a backend that keeps two copies
    /// (as the log and Audit Log file backends keep two slots) should pass
    /// over this one for the other. `false` for a sound snapshot this build
    /// must not take ([`UnknownVersion`](SnapshotErrorKind::UnknownVersion),
    /// [`UnknownKind`](SnapshotErrorKind::UnknownKind),
    /// [`OtherObject`](SnapshotErrorKind::OtherObject)), where loading
    /// should stop rather than fall back to an older copy, and for an
    /// encode refusal.
    pub const fn is_damage(&self) -> bool {
        matches!(
            self.kind,
            SnapshotErrorKind::Truncated
                | SnapshotErrorKind::NotThisFormat
                | SnapshotErrorKind::TooLarge
                | SnapshotErrorKind::Damaged
        )
    }

    /// The stack's error for this one, its message led by `format` (such as
    /// "log snapshot"): [`Error::OutOfRange`] for an encode refusal,
    /// [`Error::Encoding`] for anything found in stored octets.
    pub(crate) fn into_error(self, format: &str) -> Error {
        let message = format!("{format} {self}");
        match self.kind {
            SnapshotErrorKind::OutOfRange => Error::OutOfRange(message),
            _ => Error::Encoding(message),
        }
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = self.field {
            write!(f, "{field} ")?;
        }
        match self.kind {
            SnapshotErrorKind::UnknownVersion(version) => {
                write!(f, "schema version {version} is unsupported")
            }
            SnapshotErrorKind::UnknownKind(kind) => write!(f, "{} ({kind})", self.detail),
            _ => f.write_str(self.detail),
        }
    }
}

impl core::error::Error for SnapshotError {}
