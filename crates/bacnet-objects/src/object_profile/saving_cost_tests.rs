//! A staged Tags write costs what it changes (#2546): the stage counts the
//! octets the Tags encode to instead of encoding them for each write, makes
//! each write in place with an undo, and each claim makes its write again on
//! the state served. Each of these answers as the copy it replaces did.

use super::super::test_support::{Memory, apply, oid, staged};
use super::super::{MAX_TAGS, check_tag};
use super::*;
use crate::snapshot::framing::HEADER_LEN;
use crate::snapshot::tags::MAX_LEN;
use bacnet_encoding::constructed::encode_name_value;
use proptest::prelude::*;
use std::sync::Arc;

/// The octets a body holds beside the Tags: the presence octet, and a
/// Description's length and text.
fn beside(description: Option<&str>) -> usize {
    1 + description.map_or(0, |text| 4 + text.len())
}

/// One tag whose encoding takes `octets`.
fn tag_of(octets: usize) -> BACnetNameValue {
    let mut name_len = octets.saturating_sub(2);
    loop {
        let tag = BACnetNameValue::semantic("x".repeat(name_len));
        let len = tags_len(core::slice::from_ref(&tag), &mut BytesMut::new()).unwrap();
        match len.cmp(&octets) {
            core::cmp::Ordering::Equal => return tag,
            core::cmp::Ordering::Greater => name_len -= len - octets,
            core::cmp::Ordering::Less => name_len += octets - len,
        }
    }
}

/// Two tags whose encodings take `octets` together.
fn tags_of(octets: usize) -> Vec<BACnetNameValue> {
    vec![tag_of(octets - 2), BACnetNameValue::semantic("")]
}

/// A result as a test compares it: an error by what it says.
fn answer<T>(result: &Result<T, Error>) -> Result<(), String> {
    result
        .as_ref()
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

fn framed(tags: &[BACnetNameValue]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for tag in tags {
        encode_name_value(&mut octets, tag).unwrap();
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

/// What a copy of `tags` answers for a write, as each stage step worked it
/// out before #2546: the write made on the copy, then the copy's whole body
/// encoded to judge the file cap.
fn on_a_copy(
    tags: &[BACnetNameValue],
    index: Option<u32>,
    value: &PropertyValue,
    description: Option<&str>,
) -> Result<Vec<BACnetNameValue>, Error> {
    let mut copy = match index {
        Some(_) => tags.to_vec(),
        None => Vec::new(),
    };
    write_tags(&mut copy, index, value)?;
    fits(description, Some(&copy))?;
    Ok(copy)
}

/// Make the write on `working`, and check it answered and left what a copy
/// does, and that it still counts the octets its Tags encode to.
fn check(
    working: &mut Working,
    index: Option<u32>,
    value: &PropertyValue,
    description: Option<&str>,
) -> Result<(), Error> {
    let before = working.tags.clone();
    let expected = on_a_copy(&before, index, value, description);
    let made = working.write(index, value, description, &mut BytesMut::new());
    assert_eq!(answer(&made), answer(&expected));
    assert_eq!(working.tags, expected.unwrap_or(before));
    let len = tags_len(&working.tags, &mut BytesMut::new()).unwrap();
    assert_eq!(working.len, len, "the counted octets");
    made
}

/// The counted length judges the file cap as encoding the body does: at
/// the cap, one octet past it, and with no Tags or none written, beside no
/// Description, a short one, one at its cap and one past it.
#[test]
fn the_counted_octets_judge_the_cap_as_the_encoded_body() {
    let room = MAX_LEN - HEADER_LEN;
    let at_cap = "d".repeat(16 * 1024);
    let past_cap = "d".repeat(16 * 1024 + 1);
    for description in [
        None,
        Some(""),
        Some("lobby"),
        Some(&*at_cap),
        Some(&*past_cap),
    ] {
        let fill = room - beside(description).min(room);
        let lists = [
            None,
            Some(Vec::new()),
            Some(tags_of(fill - 1)),
            Some(tags_of(fill)),
            Some(tags_of(fill + 1)),
        ];
        for tags in lists {
            let counted = tags
                .as_deref()
                .map(|tags| tags_len(tags, &mut BytesMut::new()).unwrap());
            let counted_answer = answer(&fits_len(description, counted));
            let encoded = answer(&fits(description, tags.as_deref()));
            assert_eq!(counted_answer, encoded, "{counted:?}");
        }
    }
    // The lists above are the octets they were built to take.
    assert_eq!(
        tags_len(&tags_of(77_777), &mut BytesMut::new()).unwrap(),
        77_777
    );
}

/// At the cap a write that leaves the Tags one octet longer than the room
/// beside the Description is refused and undone, and one that fills the
/// room exactly is made: an element, a size, and a size down from Tags
/// served past the cap.
#[test]
fn a_write_at_the_cap_is_made_or_undone_as_on_a_copy() {
    let description = Some("lobby");
    let room = MAX_LEN - HEADER_LEN - beside(description);
    let empty = tags_len(&[BACnetNameValue::semantic("")], &mut BytesMut::new()).unwrap();
    // Two octets short: one more empty tag fills the room, two don't fit.
    let mut working = Working::of(tags_of(room - empty), &mut BytesMut::new()).unwrap();
    let size = |size: u64| PropertyValue::Unsigned(size);
    assert!(check(&mut working, Some(0), &size(4), description).is_err());
    check(&mut working, Some(0), &size(3), description).unwrap();
    assert_eq!(working.len, room);
    // An element as long as the one it replaces fits; one octet more doesn't.
    let same = framed(&[BACnetNameValue::semantic("")]);
    let short = framed(&[BACnetNameValue::semantic("a")]);
    check(&mut working, Some(3), &same, description).unwrap();
    assert!(check(&mut working, Some(3), &short, description).is_err());
    // Served Tags past the cap: a smaller size still past it is refused, the
    // tags it would drop put back in order; one inside it is made.
    let mut tags = tags_of(room);
    tags.extend([
        BACnetNameValue::semantic("past"),
        BACnetNameValue::semantic(""),
    ]);
    let mut working = Working::of(tags, &mut BytesMut::new()).unwrap();
    assert!(check(&mut working, Some(0), &size(3), description).is_err());
    check(&mut working, Some(0), &size(2), description).unwrap();
    // A whole array that fills the room is made; one octet more isn't.
    check(&mut working, None, &framed(&tags_of(room)), description).unwrap();
    assert!(check(&mut working, None, &framed(&tags_of(room + 1)), description).is_err());
    // A refused value or index changes nothing.
    assert!(check(&mut working, Some(9), &short, description).is_err());
    assert!(check(&mut working, Some(1), &PropertyValue::Null, description).is_err());
    assert!(
        check(
            &mut working,
            Some(0),
            &PropertyValue::Boolean(true),
            description
        )
        .is_err()
    );
    assert!(
        check(
            &mut working,
            Some(0),
            &size(MAX_TAGS as u64 + 1),
            description
        )
        .is_err()
    );
}

/// A Description staged after a Tags write the stage refused is judged
/// beside the Tags storage holds, as one made where it is is, not beside
/// the copy of the Tags served the refused write was worked out on: here
/// storage holds none, and the Tags served fill the file.
#[test]
fn a_description_after_a_refused_tags_write_fits_beside_the_saved_tags() {
    let store = Arc::new(Memory::default());
    let mut state = ProfileState::persistent(oid(), store.clone(), true).unwrap();
    let full = tags_of(MAX_LEN - HEADER_LEN - beside(None));
    let profile = ObjectProfile {
        tags: Some(full),
        ..ObjectProfile::default()
    };
    state.provision(profile).unwrap();
    let text = "d".repeat(16 * 1024);
    let writes = [
        // A NULL refused as the wrong datatype: the steps go on past it.
        Kept::Tags(Some(1), PropertyValue::Null).pending(),
        Kept::Description(PropertyValue::CharacterString(text.clone())).pending(),
    ];
    let wait = staged(state.stage_writes(&writes));
    wait.block();
    let saved = TagsSnapshot {
        tags: None,
        description: Some(text),
    };
    assert_eq!(store.saved(), Some(saved));
    state.release_staged_write(&wait);
}

/// Only an element past the Tags served makes a staged Tags write stale.
#[test]
fn only_an_element_past_the_tags_served_is_unmakeable() {
    let served = [
        BACnetNameValue::semantic("a"),
        BACnetNameValue::semantic("b"),
    ];
    let cases = [
        (P::TAGS, Some(3), Some(&served[..]), true),
        (P::TAGS, Some(2), Some(&served[..]), false),
        (P::TAGS, Some(1), None, true),
        (P::TAGS, Some(0), None, false),
        (P::TAGS, None, None, false),
        (P::DESCRIPTION, Some(5), Some(&served[..]), false),
    ];
    for (property, index, served, stale) in cases {
        assert_eq!(unmakeable(property, index, served), stale, "{index:?}");
    }
}

/// A name whose encoding puts three of them past the 1 MiB file cap and two
/// inside it, a short one, or one a write refuses.
fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => (0usize..4).prop_map(|len| "n".repeat(len)),
        2 => (349_000usize..351_000).prop_map(|len| "n".repeat(len)),
        2 => (522_000usize..526_000).prop_map(|len| "n".repeat(len)),
        1 => Just("semi;colon".to_string()),
    ]
}

fn tag() -> impl Strategy<Value = BACnetNameValue> {
    (name(), proptest::option::of(0u64..9)).prop_map(|(name, value)| {
        let mut tag = BACnetNameValue::semantic(name);
        tag.value = value.map(PropertyValue::Unsigned);
        tag
    })
}

/// A Tags write: the whole array, a size, an element, or a value or index
/// a write refuses.
fn tags_write() -> impl Strategy<Value = (Option<u32>, PropertyValue)> {
    let size = prop_oneof![(0u64..6), Just(MAX_TAGS as u64 + 1)];
    prop_oneof![
        proptest::collection::vec(tag(), 0..4).prop_map(|tags| (None, framed(&tags))),
        size.prop_map(|size| (Some(0), PropertyValue::Unsigned(size))),
        (1u32..6, tag()).prop_map(|(index, tag)| (Some(index), framed(&[tag]))),
        (1u32..4).prop_map(|index| (Some(index), PropertyValue::Null)),
        Just((Some(0), PropertyValue::Boolean(true))),
    ]
}

fn description() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(prop_oneof![
        Just(String::new()),
        Just("lobby".to_string()),
        Just("d".repeat(16 * 1024)),
    ])
}

/// A Description write: its text, one past the cap, or a NULL, which storage
/// doesn't keep.
fn description_write() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        description().prop_map(|text| PropertyValue::CharacterString(text.unwrap_or_default())),
        Just(PropertyValue::CharacterString("d".repeat(16 * 1024 + 1))),
        Just(PropertyValue::Null),
    ]
}

/// A kept write to an object that keeps its Description with its Tags.
#[derive(Clone, Debug)]
enum Kept {
    Tags(Option<u32>, PropertyValue),
    Description(PropertyValue),
}

fn kept() -> impl Strategy<Value = Kept> {
    prop_oneof![
        4 => tags_write().prop_map(|(index, value)| Kept::Tags(index, value)),
        1 => description_write().prop_map(Kept::Description),
    ]
}

impl Kept {
    fn pending(&self) -> PendingWrite {
        let (property, value) = match self {
            Self::Tags(_, value) => (P::TAGS, value),
            Self::Description(value) => (P::DESCRIPTION, value),
        };
        let array_index = match self {
            Self::Tags(index, _) => *index,
            Self::Description(_) => None,
        };
        PendingWrite {
            property,
            array_index,
            value: value.clone(),
        }
    }

    /// Make the write as its object would, the Description it serves in
    /// `description`.
    fn make(&self, state: &mut ProfileState, description: &mut String) -> Result<(), Error> {
        match self {
            Self::Tags(..) => apply(state, &self.pending()),
            Self::Description(value) => match state.write_description(description, value) {
                Some(made) => made,
                // Kept nowhere: the object makes it where it is.
                None => Ok(()),
            },
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Each write made in place answers and leaves what the same write
    /// made on a copy does, Tags past the cap and refusals included, and
    /// the counted octets stay those the Tags encode to.
    #[test]
    fn writes_made_in_place_answer_as_on_a_copy(
        served in proptest::collection::vec(tag(), 0..4),
        description in description(),
        writes in proptest::collection::vec(tags_write(), 1..8),
    ) {
        let served: Vec<_> = served.into_iter().filter(|tag| check_tag(tag).is_ok()).collect();
        let mut working = Working::of(served, &mut BytesMut::new()).unwrap();
        for (index, value) in &writes {
            let _ = check(&mut working, *index, value, description.as_deref());
        }
    }

    /// A staged request answers, serves and leaves storage holding what the
    /// same writes made one at a time do, its Description beside its Tags.
    #[test]
    fn a_staged_request_leaves_what_its_writes_one_at_a_time_do(
        served in proptest::collection::vec(tag(), 0..4),
        saved in description(),
        writes in proptest::collection::vec(kept(), 1..8),
    ) {
        let served: Vec<_> = served.into_iter().filter(|tag| check_tag(tag).is_ok()).collect();
        let run = |stage: bool| {
            let store = Arc::new(Memory::default());
            *store.saved.lock().unwrap() = saved.clone().map(|text| TagsSnapshot {
                tags: None,
                description: Some(text),
            });
            let mut state = ProfileState::persistent(oid(), store.clone(), true).unwrap();
            state.provision(ObjectProfile {
                tags: Some(served.clone()),
                ..ObjectProfile::default()
            }).unwrap();
            let mut description = state.saved_description().unwrap_or_default().to_owned();
            let pending: Vec<_> = writes.iter().map(Kept::pending).collect();
            let wait = stage.then(|| state.stage_writes(&pending)).and_then(|step| match step {
                StageStep::Skip => None,
                step => Some(staged(step)),
            });
            if let Some(wait) = &wait {
                wait.block();
            }
            let mut made = Vec::new();
            for write in &writes {
                let result = write.make(&mut state, &mut description);
                let stop = result.is_err() && write.pending().value != PropertyValue::Null;
                made.push((answer(&result), state.profile().tags.clone(), description.clone()));
                if stop {
                    break;
                }
            }
            if let Some(wait) = &wait {
                state.release_staged_write(wait);
            }
            state.wait_for_saves();
            // Storage holding none of the two loads as storage holding
            // nothing, which a request that ends refused may leave.
            (made, store.saved().unwrap_or_default())
        };
        prop_assert_eq!(run(true), run(false));
    }
}
