//! Passwords in the decode tree (#2074): DeviceCommunicationControl and
//! ReinitializeDevice carry one, which the named fields show as hidden and
//! the tag walk hides wherever it may sit.

use bacnet_encoding::tags::{Tag, TagClass, decode_tag};
use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Doc, Field, Params};

/// Where a password-bearing request keeps its password.
#[derive(Debug, Clone, Copy)]
pub(super) struct Secret {
    /// The context tag of the password.
    tag: u8,
    /// The context tags of the fields that may come before it, in order,
    /// each with its largest content length: the same fields and sizes the
    /// capture's redactor walks past.
    before: &'static [(u8, u32)],
}

impl Params {
    /// Where a password-bearing request keeps its password.
    pub(super) fn secret(self) -> Option<Secret> {
        match self {
            // Time duration (leniently up to 4 octets), then enable/disable.
            Self::Request(Confirmed::DEVICE_COMMUNICATION_CONTROL) => Some(Secret {
                tag: 2,
                before: &[(0, 4), (1, 1)],
            }),
            // The state to reinitialize to.
            Self::Request(Confirmed::REINITIALIZE_DEVICE) => Some(Secret {
                tag: 1,
                before: &[(0, 1)],
            }),
            _ => None,
        }
    }
}

/// What the password field shows, on either path.
pub(super) const PASSWORD: &str = "Password: (hidden)";

/// What any other field that may hold a password shows on the tag walk.
const MAYBE_PASSWORD: &str = "Hidden: may hold a password";

/// On the tag walk of a password-bearing request, whose decoder refused
/// it or saw only a segment: the fields before the password stay, each at
/// most once, in order and at its expected size ([`Secret::before`]). From
/// the password tag, or the first field that isn't one of them, to the end
/// every value at any depth is hidden: a password's length may claim too
/// little and leave the rest of it to read as more tags.
pub(super) fn hide_passwords(fields: &mut [Field], doc: &Doc<'_>, secret: Secret) {
    let mut next = 0;
    let mut open = true;
    for field in fields {
        let header = header(field, doc);
        if open
            && let Some(header) = header
            && header.class == TagClass::Context
            && !header.is_opening
            && !header.is_closing
            && let Some(skipped) = secret.before[next..]
                .iter()
                .position(|&(number, most)| number == header.number && header.length <= most)
        {
            next += skipped + 1;
            continue;
        }
        open = false;
        let password = header.is_some_and(|header| {
            header.class == TagClass::Context
                && header.number == secret.tag
                && !header.is_opening
                && !header.is_closing
        });
        if password {
            field.label = PASSWORD.into();
            field.device_text = false;
        } else {
            hide(field, doc);
        }
    }
}

/// The tag header a field starts with, when it reads as one.
fn header(field: &Field, doc: &Doc<'_>) -> Option<Tag> {
    field
        .range
        .start
        .checked_sub(doc.at)
        .and_then(|at| decode_tag(doc.data, at).ok())
        .map(|(header, _)| header)
}

/// Hide `field` and every part of it; the opening and closing tags of a
/// constructed value keep their labels, which quote nothing.
fn hide(field: &mut Field, doc: &Doc<'_>) {
    if !field.children.is_empty() {
        for child in &mut field.children {
            hide(child, doc);
        }
        return;
    }
    if header(field, doc).is_some_and(|header| header.is_opening || header.is_closing) {
        return;
    }
    field.label = MAYBE_PASSWORD.into();
    field.device_text = false;
}
