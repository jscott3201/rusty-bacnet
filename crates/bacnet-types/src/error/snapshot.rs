//! Persisted state that can't be loaded or saved (#1833, #2047):
//! [`Error::Persistence`](super::Error::Persistence) and the
//! [`SnapshotError`] it carries, which every snapshot codec of
//! `bacnet-objects` returns and its storage backends report.

use core::fmt;

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
    /// The storage behind the snapshot failed, not its octets (#1833): a
    /// save that failed, or panicked, or whose failure was already reported
    /// to an earlier caller, or a stored copy that changed while it was
    /// read. Whatever is stored may still be sound.
    BackendFailed,
}

/// Why a snapshot didn't encode, decode, load or save.
///
/// It carries only static text, so a firmware image reports it without
/// allocating; formatting it ([`Display`](fmt::Display)) is the only cost,
/// and only when the application asks. The text names the format, then the
/// field at fault when there is one, then the reason. A storage backend of
/// the application's own reports its failure as
/// [`backend_failed`](Self::backend_failed), or as
/// [`damaged`](Self::damaged) for octets no save wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotError {
    kind: SnapshotErrorKind,
    format: &'static str,
    field: Option<&'static str>,
    detail: &'static str,
}

impl SnapshotError {
    /// An error of `kind`, described by `detail`.
    pub const fn new(kind: SnapshotErrorKind, detail: &'static str) -> Self {
        Self {
            kind,
            format: "",
            field: None,
            detail,
        }
    }

    /// Damage, described by `detail`.
    pub const fn damaged(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::Damaged, detail)
    }

    /// The octets end early.
    pub const fn truncated(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::Truncated, detail)
    }

    /// A value the format can't keep, refused on encode.
    pub const fn out_of_range(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::OutOfRange, detail)
    }

    /// The storage failed, described by `detail`
    /// ([`BackendFailed`](SnapshotErrorKind::BackendFailed)).
    pub const fn backend_failed(detail: &'static str) -> Self {
        Self::new(SnapshotErrorKind::BackendFailed, detail)
    }

    /// The same error, about the field `field`.
    #[must_use]
    pub const fn in_field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }

    /// The same error, its text led by `format` (such as "log snapshot").
    #[must_use]
    pub const fn in_format(mut self, format: &'static str) -> Self {
        self.format = format;
        self
    }

    /// The same check failing as a snapshot is encoded: a refusal
    /// ([`OutOfRange`](SnapshotErrorKind::OutOfRange)), never damage, since
    /// nothing was stored.
    #[must_use]
    pub const fn refusing(mut self) -> Self {
        self.kind = SnapshotErrorKind::OutOfRange;
        self
    }

    /// The same check failing on stored octets: damage
    /// ([`Damaged`](SnapshotErrorKind::Damaged)), whatever an encode of the
    /// same snapshot would have called it.
    #[must_use]
    pub const fn damaging(mut self) -> Self {
        self.kind = SnapshotErrorKind::Damaged;
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
    /// should stop rather than fall back to an older copy, for every
    /// encode refusal ([`OutOfRange`](SnapshotErrorKind::OutOfRange)), and
    /// for a storage failure
    /// ([`BackendFailed`](SnapshotErrorKind::BackendFailed)), which says
    /// nothing of the octets.
    pub const fn is_damage(&self) -> bool {
        matches!(
            self.kind,
            SnapshotErrorKind::Truncated
                | SnapshotErrorKind::NotThisFormat
                | SnapshotErrorKind::TooLarge
                | SnapshotErrorKind::Damaged
        )
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.format.is_empty() {
            write!(f, "{} ", self.format)?;
        }
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

#[cfg(test)]
mod tests {
    use super::super::Error;
    use super::*;

    /// A persisted-state error prints the snapshot's text alone, with no
    /// category in front, and keeps its kind (#1833).
    #[test]
    fn persistence_errors_print_the_snapshot_text_and_keep_the_kind() {
        let damaged = SnapshotError::damaged("checksum mismatch")
            .in_field("rows")
            .in_format("log snapshot");
        let error = Error::from(damaged);
        assert_eq!(error.to_string(), "log snapshot rows checksum mismatch");
        assert!(matches!(error, Error::Persistence(e) if e.is_damage()));
        let backend = Error::from(SnapshotError::backend_failed("storage unavailable"));
        assert_eq!(backend.to_string(), "storage unavailable");
        assert!(matches!(
            backend,
            Error::Persistence(e)
                if e.kind() == SnapshotErrorKind::BackendFailed && !e.is_damage()
        ));
        let version = SnapshotError::new(SnapshotErrorKind::UnknownVersion(9), "");
        assert_eq!(
            Error::from(version.in_format("Tags file")).to_string(),
            "Tags file schema version 9 is unsupported"
        );
    }

    /// Which kinds are damage, so a two-slot backend passes over the copy
    /// for the other one: pinned, as the fallback acts on it.
    #[test]
    fn only_damage_kinds_are_damage() {
        for (kind, damage) in [
            (SnapshotErrorKind::Truncated, true),
            (SnapshotErrorKind::NotThisFormat, true),
            (SnapshotErrorKind::TooLarge, true),
            (SnapshotErrorKind::Damaged, true),
            (SnapshotErrorKind::UnknownVersion(2), false),
            (SnapshotErrorKind::UnknownKind(7), false),
            (SnapshotErrorKind::OtherObject, false),
            (SnapshotErrorKind::OutOfRange, false),
            (SnapshotErrorKind::BackendFailed, false),
        ] {
            assert_eq!(
                SnapshotError::new(kind, "x").is_damage(),
                damage,
                "{kind:?}"
            );
        }
    }

    /// A persisted-state error takes no Reject reason: a responder answers
    /// it as it answered the text it replaces.
    #[test]
    fn persistence_errors_take_no_reject_reason() {
        let error = Error::from(SnapshotError::damaged("x"));
        assert_eq!(error.reject_reason(), None);
        assert!(matches!(error.into_request_reject(), Error::Persistence(_)));
    }
}
