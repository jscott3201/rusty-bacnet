//! Passwords in the decode tree (#2074): DeviceCommunicationControl and
//! ReinitializeDevice carry one, which the named fields show as hidden and
//! the tag walk hides wherever it may sit.

use bacnet_encoding::tags::{Tag, TagClass};
use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Field, Params, Walk};

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

impl Secret {
    /// Whether `tag` is a context primitive, as every field before the
    /// password and the password itself are.
    fn primitive(tag: &Tag) -> bool {
        tag.class == TagClass::Context && !tag.is_opening && !tag.is_closing
    }

    /// Whether `tag` is the password's.
    fn is_password(self, tag: &Tag) -> bool {
        Self::primitive(tag) && tag.number == self.tag
    }
}

/// What the password field shows, on either path.
pub(super) const PASSWORD: &str = "Password: (hidden)";

/// What the octets from where hiding starts show on the tag walk.
const MAYBE_PASSWORD: &str = "Hidden: may hold a password";

/// Why a password-bearing request's decoder refused it: hidden, since its
/// reason can quote the password's octets (their character set, or how
/// many of them decode).
pub(super) const REASON_HIDDEN: &str = "reason hidden (it may quote the password)";

/// On the tag walk of a password-bearing request, whose decoder refused
/// it or saw only a segment: the fields before the password stay, each at
/// most once, in order and at its expected size ([`Secret::before`]), and
/// so does a fault the walk stopped at among them, whose text quotes only
/// the octets at the fault. Hiding starts at the password, which shows as
/// hidden, or at the first field that isn't one of them; from there to
/// `end` the octets are one hidden leaf. A password's length may claim too
/// little and leave the rest of it to read as more tags, whose numbers,
/// nesting, faults and ranges would each quote its octets.
///
/// `fields` are the walk's top-level fields, one for each of its elements
/// in order, then the leaf of a fault outside them, if any.
pub(super) fn hide_passwords(fields: &mut Vec<Field>, walk: &Walk<'_>, secret: Secret, end: usize) {
    let mut next = 0;
    let mut cut = None;
    for (index, element) in walk.elements.iter().enumerate() {
        let tag = element.tag;
        if Secret::primitive(&tag)
            && let Some(skipped) = secret.before[next..]
                .iter()
                .position(|&(number, most)| number == tag.number && tag.length <= most)
        {
            next += skipped + 1;
            continue;
        }
        cut = Some(if secret.is_password(&tag) {
            index + 1
        } else {
            index
        });
        if let Some(field) = fields.get_mut(index).filter(|_| secret.is_password(&tag)) {
            field.label = PASSWORD.into();
            field.device_text = false;
        }
        break;
    }
    // Contents cut short at the password's own tag: the password.
    let at_password = walk
        .top_stop()
        .and_then(|stop| stop.tag())
        .is_some_and(|tag| secret.is_password(&tag));
    if cut.is_none() && at_password {
        let index = walk.elements.len();
        if let Some(field) = fields.get_mut(index) {
            field.label = PASSWORD.into();
            field.device_text = false;
        }
        cut = Some(index + 1);
    }
    let Some(cut) = cut else {
        return;
    };
    // The top-level fields tile the parameters, so hiding starts where
    // the first field cut starts.
    let start = fields.get(cut).map(|field| field.range.start);
    fields.truncate(cut);
    if let Some(start) = start.filter(|start| *start < end) {
        fields.push(Field::leaf(MAYBE_PASSWORD, start..end));
    }
}
