//! Table 12-63 and Coercion Rules 1 to 6, one written datatype at a time.
use super::*;
use bacnet_types::primitives::{Date, Time};
use MemberDatatype as D;
use PropertyValue as V;

const ALL: [MemberDatatype; 16] = [
    D::Unknown,
    D::Boolean,
    D::Unsigned,
    D::Integer,
    D::Real,
    D::Double,
    D::OctetString,
    D::CharacterString,
    D::BitString,
    D::Enumerated,
    D::Date,
    D::Time,
    D::ObjectIdentifier,
    D::LightingCommand,
    D::ColorCommand,
    D::XyColor,
];

fn date() -> V {
    V::Date(Date {
        year: 126,
        month: 10,
        day: 3,
        day_of_week: 6,
    })
}

fn time() -> V {
    V::Time(Time {
        hour: 7,
        minute: 30,
        second: 0,
        hundredths: 0,
    })
}

fn bits() -> V {
    V::BitString {
        unused_bits: 4,
        data: vec![0xA0],
    }
}

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

/// Operation 1 with a target level of 50.0, framed in [0].
const LIGHTING: [u8; 9] = [0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F];

fn coerce(value: &V, target: MemberDatatype) -> Option<V> {
    coerce_channel_value(value, target).ok()
}

/// The targets `value` passes to unchanged; every other target fails, unless
/// `converted` names it.
fn assert_row(value: V, unchanged: &[MemberDatatype], converted: &[(MemberDatatype, V)]) {
    for target in ALL {
        let expected = converted
            .iter()
            .find(|(to, _)| *to == target)
            .map(|(_, to)| to.clone())
            .or_else(|| unchanged.contains(&target).then(|| value.clone()));
        assert_eq!(coerce(&value, target), expected, "{value:?} to {target:?}");
    }
}

#[test]
fn member_datatype_follows_the_value_the_property_holds() {
    fn of(property: PropertyIdentifier, current: Option<&V>) -> MemberDatatype {
        MemberDatatype::of(ObjectType::ANALOG_VALUE, property, current)
    }
    let pv = PropertyIdentifier::PRESENT_VALUE;
    assert_eq!(of(pv, Some(&V::Real(1.0))), D::Real);
    assert_eq!(of(pv, Some(&V::Enumerated(1))), D::Enumerated);
    assert_eq!(of(pv, Some(&V::Signed(1))), D::Integer);
    assert_eq!(of(pv, Some(&bits())), D::BitString);
    assert_eq!(of(pv, Some(&V::Null)), D::Unknown);
    assert_eq!(of(pv, Some(&V::List(vec![]))), D::Unknown);
    assert_eq!(of(pv, None), D::Unknown);
    // Two REALs are no xy colour outside a Color object.
    assert_eq!(
        of(pv, Some(&V::List(vec![V::Real(0.1), V::Real(0.2)]))),
        D::Unknown
    );
    assert_eq!(
        of(
            PropertyIdentifier::LIGHTING_COMMAND,
            Some(&V::OctetString(vec![]))
        ),
        D::LightingCommand
    );
}

#[test]
fn colour_properties_have_their_addendum_datatypes() {
    use ObjectType as O;
    use PropertyIdentifier as P;
    for (object_type, property, datatype) in [
        (O::COLOR, P::PRESENT_VALUE, D::XyColor),
        (O::COLOR, P::TRACKING_VALUE, D::XyColor),
        (O::COLOR, P::DEFAULT_COLOR, D::XyColor),
        (O::COLOR, P::COLOR_COMMAND, D::ColorCommand),
        (O::COLOR_TEMPERATURE, P::COLOR_COMMAND, D::ColorCommand),
        // A Color Temperature's Present_Value is an Unsigned.
        (O::COLOR_TEMPERATURE, P::PRESENT_VALUE, D::Unknown),
    ] {
        assert_eq!(MemberDatatype::of(object_type, property, None), datatype);
    }
    assert_eq!(
        MemberDatatype::of(
            O::COLOR_TEMPERATURE,
            P::PRESENT_VALUE,
            Some(&V::Unsigned(4000))
        ),
        D::Unsigned
    );
}

#[test]
fn null_passes_to_everything_but_a_constructed_datatype() {
    let primitive: Vec<_> = ALL
        .into_iter()
        .filter(|target| !matches!(target, D::LightingCommand | D::ColorCommand | D::XyColor))
        .collect();
    assert_row(V::Null, &primitive, &[]);
}

#[test]
fn boolean_row_rule_2() {
    assert_row(
        V::Boolean(true),
        &[D::Unknown, D::Boolean],
        &[
            (D::Unsigned, V::Unsigned(1)),
            (D::Integer, V::Signed(1)),
            (D::Real, V::Real(1.0)),
            (D::Double, V::Double(1.0)),
            (D::Enumerated, V::Enumerated(1)),
        ],
    );
    assert_eq!(coerce(&V::Boolean(false), D::Real), Some(V::Real(0.0)));
    assert_eq!(
        coerce(&V::Boolean(false), D::Enumerated),
        Some(V::Enumerated(0))
    );
}

#[test]
fn unsigned_row_rules_1_and_3() {
    let oid = av(5);
    let raw = u64::from(u32::from_be_bytes(oid.encode()));
    assert_row(
        V::Unsigned(raw),
        &[D::Unknown, D::Unsigned],
        &[
            (D::Boolean, V::Boolean(true)),
            (D::Integer, V::Signed(raw as i32)),
            (D::Real, V::Real(raw as f32)),
            (D::Double, V::Double(raw as f64)),
            (D::Enumerated, V::Enumerated(raw as u32)),
            (D::ObjectIdentifier, V::ObjectIdentifier(oid)),
        ],
    );
    assert_eq!(coerce(&V::Unsigned(0), D::Boolean), Some(V::Boolean(false)));
    // Rule 3: an INTEGER takes at most 2147483647.
    assert_eq!(
        coerce(&V::Unsigned(2_147_483_647), D::Integer),
        Some(V::Signed(i32::MAX))
    );
    assert_eq!(coerce(&V::Unsigned(2_147_483_648), D::Integer), None);
    // Rule 3 bounds the value at 2147483647 for REAL and Double as well.
    for target in [D::Integer, D::Real, D::Double] {
        assert_eq!(
            coerce(&V::Unsigned(2_147_483_648), target),
            None,
            "{target:?}"
        );
    }
    assert_eq!(
        coerce(&V::Unsigned(2_147_483_647), D::Double),
        Some(V::Double(2_147_483_647.0))
    );
    // Rounded to a REAL's precision, not refused.
    assert_eq!(
        coerce(&V::Unsigned(16_777_217), D::Real),
        Some(V::Real(16_777_216.0))
    );
    // Past 32 bits, neither an ENUMERATED nor an object identifier holds it.
    let wide = V::Unsigned(u64::from(u32::MAX) + 1);
    assert_eq!(coerce(&wide, D::Enumerated), None);
    assert_eq!(coerce(&wide, D::ObjectIdentifier), None);
    assert_eq!(coerce(&wide, D::Double), None);
    // Unsigned to ENUMERATED passes the number on; Rule 3 doesn't apply.
    assert_eq!(
        coerce(&V::Unsigned(3_000_000_000), D::Enumerated),
        Some(V::Enumerated(3_000_000_000))
    );
}

#[test]
fn integer_row_rules_1_and_4() {
    assert_row(
        V::Signed(-7),
        &[D::Unknown, D::Integer],
        &[
            (D::Boolean, V::Boolean(true)),
            (D::Real, V::Real(-7.0)),
            (D::Double, V::Double(-7.0)),
        ],
    );
    // Rule 4: an Unsigned or ENUMERATED takes 0 to 2147483647.
    assert_eq!(coerce(&V::Signed(12), D::Unsigned), Some(V::Unsigned(12)));
    assert_eq!(
        coerce(&V::Signed(12), D::Enumerated),
        Some(V::Enumerated(12))
    );
    assert_eq!(
        coerce(&V::Signed(i32::MAX), D::Unsigned),
        Some(V::Unsigned(2_147_483_647))
    );
    assert_eq!(coerce(&V::Signed(0), D::Boolean), Some(V::Boolean(false)));
    // Rounded to a REAL's precision, as under Rule 3.
    assert_eq!(
        coerce(&V::Signed(-16_777_217), D::Real),
        Some(V::Real(-16_777_216.0))
    );
}

#[test]
fn real_row_rules_1_and_5() {
    assert_row(
        V::Real(2.75),
        &[D::Unknown, D::Real],
        &[
            (D::Boolean, V::Boolean(true)),
            (D::Unsigned, V::Unsigned(2)),
            (D::Integer, V::Signed(2)),
            (D::Double, V::Double(2.75)),
            (D::Enumerated, V::Enumerated(2)),
        ],
    );
    assert_eq!(coerce(&V::Real(0.0), D::Boolean), Some(V::Boolean(false)));
    assert_eq!(coerce(&V::Real(-2.75), D::Integer), Some(V::Signed(-2)));
    // Rule 5: an Unsigned or ENUMERATED takes 0 to 2147483000, an INTEGER
    // the same magnitude either side of zero.
    for target in [D::Unsigned, D::Enumerated] {
        assert_eq!(coerce(&V::Real(-0.5), target), None, "{target:?}");
        assert_eq!(
            coerce(&V::Real(2_147_483_648.0), target),
            None,
            "{target:?}"
        );
        assert_eq!(coerce(&V::Real(f32::NAN), target), None, "{target:?}");
    }
    assert_eq!(coerce(&V::Real(-2_147_483_648.0), D::Integer), None);
    // The REALs nearest the bound: 2147482880 is inside, 2147483008 past it.
    assert_eq!(
        coerce(&V::Real(2_147_482_880.0), D::Integer),
        Some(V::Signed(2_147_482_880))
    );
    assert_eq!(coerce(&V::Real(2_147_483_008.0), D::Unsigned), None);
}

#[test]
fn double_row_rules_1_and_6() {
    assert_row(
        V::Double(9.5),
        &[D::Unknown, D::Double],
        &[
            (D::Boolean, V::Boolean(true)),
            (D::Unsigned, V::Unsigned(9)),
            (D::Integer, V::Signed(9)),
            (D::Real, V::Real(9.5)),
            (D::Enumerated, V::Enumerated(9)),
        ],
    );
    // Rule 6 bounds.
    assert_eq!(
        coerce(&V::Double(2_147_483_000.0), D::Unsigned),
        Some(V::Unsigned(2_147_483_000))
    );
    assert_eq!(coerce(&V::Double(2_147_483_001.0), D::Unsigned), None);
    assert_eq!(coerce(&V::Double(-2_147_483_001.0), D::Integer), None);
    assert_eq!(coerce(&V::Double(1.0e39), D::Real), None);
    assert_eq!(coerce(&V::Double(-1.0e39), D::Real), None);
    assert_eq!(coerce(&V::Double(3.0e38), D::Real), Some(V::Real(3.0e38)));
    assert_eq!(
        coerce(&V::Double(f64::from(f32::MAX)), D::Real),
        Some(V::Real(f32::MAX))
    );
}

#[test]
fn nan_and_infinities_fail_every_conversion_with_a_range() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for (from, label) in [
            (V::Real(value as f32), "REAL"),
            (V::Double(value), "Double"),
        ] {
            for target in [D::Unsigned, D::Integer, D::Enumerated] {
                assert_eq!(coerce(&from, target), None, "{label} {value} to {target:?}");
            }
        }
        assert_eq!(
            coerce(&V::Double(value), D::Real),
            None,
            "Double {value} to REAL"
        );
    }
    // No range applies: a REAL widens to a Double, and a value goes to its
    // own datatype, as it is.
    let Some(V::Double(widened)) = coerce(&V::Real(f32::NAN), D::Double) else {
        panic!("a REAL NaN widens to a Double");
    };
    assert!(widened.is_nan());
    assert_eq!(
        coerce(&V::Real(f32::INFINITY), D::Double),
        Some(V::Double(f64::INFINITY))
    );
    assert_eq!(
        coerce(&V::Double(f64::NEG_INFINITY), D::Double),
        Some(V::Double(f64::NEG_INFINITY))
    );
    // Rule 1: NaN isn't zero.
    assert_eq!(
        coerce(&V::Double(f64::NAN), D::Boolean),
        Some(V::Boolean(true))
    );
}

#[test]
fn enumerated_row_rules_1_and_3() {
    assert_row(
        V::Enumerated(3),
        &[D::Unknown, D::Enumerated],
        &[
            (D::Boolean, V::Boolean(true)),
            (D::Unsigned, V::Unsigned(3)),
            (D::Integer, V::Signed(3)),
            (D::Real, V::Real(3.0)),
            (D::Double, V::Double(3.0)),
        ],
    );
    // Rule 3 bounds an ENUMERATED at 2147483647 for each numeric target.
    for target in [D::Integer, D::Real, D::Double] {
        assert_eq!(
            coerce(&V::Enumerated(2_147_483_648), target),
            None,
            "{target:?}"
        );
        assert_eq!(coerce(&V::Enumerated(u32::MAX), target), None, "{target:?}");
    }
    assert_eq!(
        coerce(&V::Enumerated(u32::MAX), D::Unsigned),
        Some(V::Unsigned(u32::MAX.into()))
    );
}

#[test]
fn string_date_time_rows_pass_only_to_their_own_datatype() {
    assert_row(
        V::OctetString(vec![1, 2]),
        &[D::Unknown, D::OctetString],
        &[],
    );
    assert_row(
        V::CharacterString("x".into()),
        &[D::Unknown, D::CharacterString],
        &[],
    );
    assert_row(bits(), &[D::Unknown, D::BitString], &[]);
    assert_row(date(), &[D::Unknown, D::Date], &[]);
    assert_row(time(), &[D::Unknown, D::Time], &[]);
}

#[test]
fn object_identifier_row_passes_to_unsigned_as_its_number() {
    let oid = av(5);
    assert_row(
        V::ObjectIdentifier(oid),
        &[D::Unknown, D::ObjectIdentifier],
        &[(
            D::Unsigned,
            V::Unsigned(u32::from_be_bytes(oid.encode()).into()),
        )],
    );
}

#[test]
fn lighting_command_goes_to_a_lighting_command_member_only() {
    let value = V::ApplicationData(LIGHTING.to_vec());
    // The member gets the SEQUENCE inside the [0] tag.
    assert_row(
        value,
        &[],
        &[(
            D::LightingCommand,
            V::ApplicationData(LIGHTING[1..8].to_vec()),
        )],
    );
}

/// x 0.5 (0x3F000000) and y 0.25 (0x3E800000), framed in [1].
const XY: [u8; 12] = [
    0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
];

/// STEP_UP_CCT (4) by 100 K, framed in [2].
const STEP_UP: [u8; 6] = [0x2E, 0x09, 0x04, 0x59, 0x64, 0x2F];

#[test]
fn an_xy_colour_goes_to_an_xy_colour_member_only() {
    // The member gets the two REALs a WriteProperty of the colour decodes to.
    assert_row(
        V::ApplicationData(XY.to_vec()),
        &[],
        &[(D::XyColor, V::List(vec![V::Real(0.5), V::Real(0.25)]))],
    );
}

#[test]
fn a_colour_command_goes_to_a_colour_command_member_only() {
    // The member gets the SEQUENCE inside the [2] tag.
    assert_row(
        V::ApplicationData(STEP_UP.to_vec()),
        &[],
        &[(D::ColorCommand, V::ApplicationData(STEP_UP[1..5].to_vec()))],
    );
}

#[test]
fn malformed_constructed_values_go_nowhere() {
    for octets in [
        &XY[..11],
        &[0x2E, 0x09, 0x04, 0x59, 0x64][..],
        &[0x3E, 0x09, 0x04, 0x3F][..],
        &[&STEP_UP[..], &[0x00]].concat()[..],
    ] {
        assert_row(V::ApplicationData(octets.to_vec()), &[], &[]);
    }
}
