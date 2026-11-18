//! Passwords in the decode tree (#2074, #2618): DeviceCommunicationControl
//! and ReinitializeDevice carry one, which the named fields show as hidden.
//!
//! Where such a request's decoder refused it, or the tree holds only its
//! first segment, the tag walk shows only the fields before the password
//! that match the service's layout exactly ([`Secret::layout`]): its
//! top-level elements while each is the next field the layout expects, in
//! order and at most once, an optional one possibly absent, a context
//! primitive of that field's tag with 1 to [`Before::most`] octets of
//! contents. The first element that is anything else (the password among
//! them), and everything after it to the end of the parameters (further
//! elements, a fault the walk stopped at, whatever its text would say), is
//! one hidden leaf: a field of the wrong length may have taken in the
//! password's octets, and a password that claims too few leaves the rest of
//! it to read as tags.
//!
//! The named fields show only when the walk's elements are the layout's
//! and nothing more ([`Secret::whole`]), and whether the decoder takes them
//! is decided with the password's contents replaced by a fixed valid string
//! of the same length ([`Secret::masked`]), so the layout decides it, not
//! whether the password is valid.

use std::borrow::Cow;

use bacnet_encoding::tags::{Tag, TagClass};
use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Element, Field, Params, Walk};

/// A field a password-bearing request may carry before its password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Before {
    /// Its context tag.
    pub(super) tag: u8,
    /// The most contents octets a well-formed one has: an Unsigned16's two,
    /// or the one octet every value of its enumeration encodes in.
    pub(super) most: u32,
    /// Whether the request must carry it.
    pub(super) required: bool,
}

/// Where a password-bearing request keeps its password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Secret {
    /// The context tag of the password.
    pub(super) tag: u8,
    /// The fields before it, in order.
    pub(super) before: &'static [Before],
}

/// DeviceCommunicationControl: an optional time duration (Unsigned16), the
/// enable-disable choice, then the password, context tag 2.
pub(super) const DCC: Secret = Secret {
    tag: 2,
    before: &[
        Before {
            tag: 0,
            most: 2,
            required: false,
        },
        Before {
            tag: 1,
            most: 1,
            required: true,
        },
    ],
};

/// ReinitializeDevice: the state to reinitialize to, then the password,
/// context tag 1.
pub(super) const REINIT: Secret = Secret {
    tag: 1,
    before: &[Before {
        tag: 0,
        most: 1,
        required: true,
    }],
};

impl Params {
    /// Where a password-bearing request keeps its password.
    pub(super) fn secret(self) -> Option<Secret> {
        match self {
            Self::Request(Confirmed::DEVICE_COMMUNICATION_CONTROL) => Some(DCC),
            Self::Request(Confirmed::REINITIALIZE_DEVICE) => Some(REINIT),
            _ => None,
        }
    }
}

/// How much of a request's tag walk matches its service's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Layout {
    /// How many top-level elements are the fields before the password.
    pub(super) before: usize,
    /// The password's element, when the next one is it.
    pub(super) password: Option<usize>,
}

/// Whether `tag` is a context primitive.
fn primitive(tag: &Tag) -> bool {
    tag.class == TagClass::Context && !tag.is_opening && !tag.is_closing
}

impl Secret {
    /// How much of `elements` matches the layout: the fields before the
    /// password, each the next one expected (an optional one may be
    /// absent), a context primitive of its tag with 1 to [`Before::most`]
    /// octets; then, with every required field there, the password.
    pub(super) fn layout(self, elements: &[Element<'_>]) -> Layout {
        let mut next = 0;
        let mut before = 0;
        for element in elements {
            let tag = element.tag;
            let Some(skipped) = self.before[next..]
                .iter()
                .position(|field| field.tag == tag.number)
            else {
                break;
            };
            let field = self.before[next + skipped];
            let absent = &self.before[next..next + skipped];
            if absent.iter().any(|field| field.required)
                || !primitive(&tag)
                || !(1..=field.most).contains(&tag.length)
            {
                break;
            }
            next += skipped + 1;
            before += 1;
        }
        let complete = self.before[next..].iter().all(|field| !field.required);
        let password = elements
            .get(before)
            .filter(|element| complete && primitive(&element.tag) && element.number() == self.tag)
            .map(|_| before);
        Layout { before, password }
    }

    /// Whether `walk` read the parameters whole and they are the layout's
    /// and nothing more: only then do the named fields show, since a
    /// decoder may take a field longer than the layout's (an
    /// enable-disable of four octets), which can hold the password's.
    pub(super) fn whole(self, walk: &Walk<'_>) -> bool {
        let layout = self.layout(&walk.elements);
        let read = layout.before + usize::from(layout.password.is_some());
        walk.top_stop().is_none() && read == walk.elements.len()
    }

    /// `data`, the parameters, with the password's contents (when the
    /// layout holds one) replaced by a valid string of the same length:
    /// UTF-8 `x`s after the character set.
    pub(super) fn masked<'a>(self, data: &'a [u8], walk: &Walk<'_>) -> Cow<'a, [u8]> {
        let Some(index) = self.layout(&walk.elements).password else {
            return Cow::Borrowed(data);
        };
        let contents = walk.elements[index].contents_range();
        let mut masked = data.to_vec();
        if let Some(octets) = masked.get_mut(contents) {
            octets.fill(b'x');
            if let Some(charset) = octets.first_mut() {
                *charset = 0;
            }
        }
        Cow::Owned(masked)
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

/// On the tag walk of a password-bearing request: the fields before the
/// password that [`Secret::layout`] matches stay, and from the first
/// element after them, the password's included, to `end` the octets are
/// one hidden leaf. The password gets no leaf of its own, since where its
/// tag seems to be and how long it claims to be can come from its octets
/// when an earlier field's length is wrong.
///
/// `fields` are the walk's top-level fields, one for each of its elements
/// in order, then the leaf of a fault outside them, if any.
pub(super) fn hide_passwords(fields: &mut Vec<Field>, walk: &Walk<'_>, secret: Secret, end: usize) {
    let shown = secret.layout(&walk.elements).before;
    // The top-level fields tile the parameters, so hiding starts where
    // the first field not shown starts.
    let start = fields.get(shown).map(|field| field.range.start);
    fields.truncate(shown);
    if let Some(start) = start.filter(|start| *start < end) {
        fields.push(Field::leaf(MAYBE_PASSWORD, start..end));
    }
}
