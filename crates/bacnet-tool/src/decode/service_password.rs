//! Passwords in the decode tree (#2074): DeviceCommunicationControl and
//! ReinitializeDevice carry one, which the named fields show as hidden and
//! the tag walk hides wherever it may sit.

use bacnet_encoding::tags::TagClass;
use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Element, Field, Params};

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
///
/// `fields` are the walk's fields of `elements`, one each in order at
/// every level. A level's fields may end with the leaf marking where the
/// walk stopped, which has no element: it names the fault and quotes no
/// octets, so it stays.
pub(super) fn hide_passwords(fields: &mut [Field], elements: &[Element<'_>], secret: Secret) {
    let mut next = 0;
    let mut open = true;
    for (field, element) in fields.iter_mut().zip(elements) {
        let tag = element.item.tag;
        let primitive = tag.class == TagClass::Context && !tag.is_opening && !tag.is_closing;
        if open
            && primitive
            && let Some(skipped) = secret.before[next..]
                .iter()
                .position(|&(number, most)| number == tag.number && tag.length <= most)
        {
            next += skipped + 1;
            continue;
        }
        open = false;
        if primitive && tag.number == secret.tag {
            field.label = PASSWORD.into();
            field.device_text = false;
        } else {
            hide(field, element);
        }
    }
}

/// Hide `field` and every part of it; a constructed value's node keeps
/// its label, which quotes nothing, as does a fault's leaf.
fn hide(field: &mut Field, element: &Element<'_>) {
    if element.is_constructed() {
        for (child, element) in field.children.iter_mut().zip(&element.children) {
            hide(child, element);
        }
        return;
    }
    field.label = MAYBE_PASSWORD.into();
    field.device_text = false;
}
