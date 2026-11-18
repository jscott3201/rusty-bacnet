//! Passwords in the decode tree (#2074, #2618): DeviceCommunicationControl
//! and ReinitializeDevice carry one.
//!
//! A request whose decoder takes it shows its named fields, the password
//! as hidden. One the decoder refuses, or the first segment of one, shows
//! its parameters as a single hidden leaf, from their first octet to the
//! end, with a fixed reason: no field, fault, nesting or range read from
//! them, since a field of the wrong length may hold the password's octets,
//! and a password that claims too few leaves the rest of it to read as
//! tags. A later segment is one opaque leaf too.
//!
//! What the tree shows then depends on the parameters' octets only through
//! whether the decoder takes them, which the password's own validity is
//! part of: more than 20 characters, a character set it doesn't read, or
//! contents that aren't valid in theirs turn named fields into the hidden
//! leaf.

use std::ops::Range;

use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Field, Params};

impl Params {
    /// Whether these parameters may carry a password.
    pub(super) fn carry_a_password(self) -> bool {
        matches!(
            self,
            Self::Request(Confirmed::DEVICE_COMMUNICATION_CONTROL | Confirmed::REINITIALIZE_DEVICE)
        )
    }
}

/// What the password field shows on the named path.
pub(super) const PASSWORD: &str = "Password: (hidden)";

/// The fields of a password-bearing request's parameters at `range` that
/// its decoder didn't read as named fields: one hidden leaf, and when the
/// decoder `refused` them, a reason that quotes nothing of them.
pub(super) fn hidden(range: Range<usize>, refused: bool) -> Vec<Field> {
    let mut fields = vec![Field::leaf("Hidden: may hold a password", range.clone())];
    if refused {
        fields.push(Field::leaf(
            "Not read as named fields: reason hidden (it may quote the password)",
            range,
        ));
    }
    fields
}
