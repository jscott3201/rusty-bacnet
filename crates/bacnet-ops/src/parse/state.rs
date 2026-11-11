//! Property states typed back: the extended choices the stack names
//! (#2472).
//!
//! The formatter writes `sc-connection-state connected` for a property state
//! carried through the extended-value choice when it names the choice. This
//! reads that text back, with the numbers that always work: a value as its
//! number (`sc-connection-state 1`) and the choice as its tag (`258 connected`,
//! `258 1`). A tag the stack doesn't name takes a number only (`260 2`). The
//! text is display and input only; the wire form is the same for every
//! spelling.

use bacnet_types::constructed::{BACnetExtendedPropertyState, NamedExtendedChoice};

/// An extended property state from its text: a choice (a name or a tag above
/// 254) and a value (a name in that choice's enumeration, or a number).
///
/// Returns `None` when the text isn't two words, the tag isn't above 254, the
/// value name isn't one the choice knows, or the value is 100000 or more.
pub fn parse_extended_property_state(text: &str) -> Option<BACnetExtendedPropertyState> {
    let mut words = text.split_whitespace();
    let (choice, value) = (words.next()?, words.next()?);
    if words.next().is_some() {
        return None;
    }
    let (named, tag) = match choice.parse::<u32>() {
        Ok(tag) => (NamedExtendedChoice::from_tag(tag), tag),
        Err(_) => {
            let named = NamedExtendedChoice::from_name(choice)?;
            (Some(named), named.tag)
        }
    };
    let value = match value.parse::<u32>() {
        Ok(number) => number,
        Err(_) => named?.value_from_name(value)?,
    };
    BACnetExtendedPropertyState::new(tag, value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::property_state_text;
    use bacnet_encoding::constructed::encode_property_state;
    use bacnet_types::constructed::{BACnetPropertyStates, NAMED_EXTENDED_CHOICES};
    use bytes::BytesMut;

    fn state(tag: u32, value: u32) -> BACnetPropertyStates {
        BACnetPropertyStates::ExtendedValue(BACnetExtendedPropertyState::new(tag, value).unwrap())
    }

    #[test]
    fn the_two_named_states_print_with_their_choice() {
        assert_eq!(
            property_state_text(&state(258, 1)),
            "sc-connection-state connected"
        );
        assert_eq!(
            property_state_text(&state(258, 3)),
            "sc-connection-state failed-to-connect"
        );
        assert_eq!(
            property_state_text(&state(259, 2)),
            "sc-hub-connector-state connected-to-failover"
        );
    }

    #[test]
    fn an_unnamed_value_of_a_named_choice_keeps_its_number() {
        assert_eq!(property_state_text(&state(258, 9)), "sc-connection-state 9");
    }

    #[test]
    fn any_other_extended_value_keeps_the_number_alone() {
        assert_eq!(property_state_text(&state(260, 2)), "extended-value 2");
        assert_eq!(property_state_text(&state(255, 7)), "extended-value 7");
    }

    /// What we print parses back to the same value, for every value of each
    /// named choice and a few beyond its table.
    #[test]
    fn printed_text_parses_back_to_the_same_state() {
        for choice in NAMED_EXTENDED_CHOICES {
            for value in 0..=8 {
                let original = BACnetExtendedPropertyState::new(choice.tag, value).unwrap();
                let text = property_state_text(&BACnetPropertyStates::ExtendedValue(original));
                assert_eq!(
                    parse_extended_property_state(&text),
                    Some(original),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn numeric_and_name_forms_agree() {
        let expected = BACnetExtendedPropertyState::new(259, 1).unwrap();
        for text in [
            "sc-hub-connector-state connected-to-primary",
            "SC_HUB_CONNECTOR_STATE CONNECTED_TO_PRIMARY",
            "sc-hub-connector-state 1",
            "259 connected-to-primary",
            "259 1",
            "  259   1 ",
        ] {
            assert_eq!(
                parse_extended_property_state(text),
                Some(expected),
                "{text}"
            );
        }
        assert_eq!(
            parse_extended_property_state("260 2"),
            Some(BACnetExtendedPropertyState::new(260, 2).unwrap())
        );
    }

    #[test]
    fn text_that_names_no_extended_state_is_refused() {
        for text in [
            "",
            "sc-connection-state",
            "sc-connection-state connected extra",
            "sc-connection-state maybe",
            "sc-connection-state connected-to-primary",
            "unknown-choice 1",
            "260 connected",
            "254 1",
            "258 100000",
            "binary-value active",
        ] {
            assert_eq!(parse_extended_property_state(text), None, "{text}");
        }
    }

    /// A named state and its numeric form are the same octets on the wire:
    /// naming changes display and input only.
    #[test]
    fn named_and_numeric_forms_encode_to_the_same_octets() {
        let octets = |text: &str| {
            let parsed = parse_extended_property_state(text).unwrap();
            let mut buf = BytesMut::new();
            encode_property_state(&mut buf, &BACnetPropertyStates::ExtendedValue(parsed)).unwrap();
            buf
        };
        assert_eq!(octets("sc-connection-state connected"), octets("258 1"));
        assert_eq!(
            octets("sc-hub-connector-state connected-to-failover"),
            octets("259 2")
        );
        // 258 * 100000 + 1 = 0x0189AD41 under context tag 63: tag 0xFC, length 4.
        assert_eq!(
            &octets("258 1")[..],
            &[0xfc, 0x3f, 0x01, 0x89, 0xad, 0x41][..]
        );
    }
}
