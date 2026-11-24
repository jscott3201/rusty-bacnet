//! A point list from a device whose settings are wrong (#2765), by
//! ReadPropertyMultiple and by ReadProperty: a setting of another datatype
//! is a problem on the record and absent from the point, so the list
//! builds; a setting the builder refuses is a problem on the record and
//! kept in the point, which the builder refuses for that same property.

use bacnet_objects::device_identity::DeviceIdentity;
use bacnet_objects::points::{build_database, build_object};
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType, Polarity};
use bacnet_types::primitives::PropertyValue;

use super::tests::oid;
use super::*;
use crate::format::RawRow;
use crate::inventory::test_lab::{
    Answer, Answering, Faults, LAB, lab_device, refusing_rpm, row, serve, stop, text,
};

use PropertyIdentifier as P;
use PropertyValue as V;

/// The rows a point of `object_type` needs, `extra` first (the first row
/// of a property counts, so `extra` can replace one), and a Priority_Array
/// when `commanded`.
fn rows(object_type: ObjectType, commanded: bool, extra: Vec<RawRow>) -> Vec<RawRow> {
    let mut rows = extra;
    rows.push(row(P::OBJECT_NAME, text("point")));
    if matches!(
        family_of(object_type),
        Some(Family::Analog | Family::Integer)
    ) {
        rows.push(row(P::UNITS, V::Enumerated(62)));
    }
    if family_of(object_type) == Some(Family::MultiState) {
        rows.push(row(P::NUMBER_OF_STATES, V::Unsigned(3)));
    }
    if commanded {
        rows.push(row(P::PRIORITY_ARRAY, V::Null));
    }
    rows
}

/// The families the sweep covers, by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Analog,
    Binary,
    MultiState,
    Integer,
}

fn family_of(object_type: ObjectType) -> Option<Family> {
    type O = ObjectType;
    Some(match object_type {
        O::ANALOG_INPUT | O::ANALOG_OUTPUT | O::ANALOG_VALUE => Family::Analog,
        O::BINARY_INPUT | O::BINARY_OUTPUT | O::BINARY_VALUE => Family::Binary,
        O::MULTI_STATE_INPUT | O::MULTI_STATE_OUTPUT | O::MULTI_STATE_VALUE => Family::MultiState,
        O::INTEGER_VALUE => Family::Integer,
        _ => return None,
    })
}

/// One fault, or none, in each setting and each property a point list
/// builder judges, on each of the ten types, a value commanded and not:
/// the record names a problem exactly when `build_object` refuses the
/// point made from it, and names the property the builder does. Non-finite
/// Reals of both signs, -0.0, the ends of the ranges and their neighbours
/// are among them.
#[test]
fn the_records_refusals_are_the_builders() {
    let reals = [
        0.0,
        -0.0,
        0.5,
        -0.5,
        1e30,
        -1e30,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];
    let pairs = [(1.0, 1.0), (2.0, 1.0), (1.0, 2.0), (-0.0, 0.0), (0.0, -0.0)];
    let mut cases: Vec<(ObjectType, Vec<RawRow>)> = Vec::new();
    for object_type in EXPORTED_TYPES {
        let family = family_of(object_type).unwrap();
        let mut faults: Vec<Vec<RawRow>> = vec![Vec::new(), vec![row(P::OBJECT_NAME, text(""))]];
        match family {
            Family::Analog => {
                for x in reals {
                    for property in [
                        P::COV_INCREMENT,
                        P::MIN_PRES_VALUE,
                        P::MAX_PRES_VALUE,
                        P::RELINQUISH_DEFAULT,
                    ] {
                        faults.push(vec![row(property, V::Real(x))]);
                    }
                }
                for (min, max) in pairs {
                    faults.push(vec![
                        row(P::MIN_PRES_VALUE, V::Real(min)),
                        row(P::MAX_PRES_VALUE, V::Real(max)),
                    ]);
                }
            }
            Family::Binary => {
                for state in [0, 1, 2, 7] {
                    faults.push(vec![row(P::RELINQUISH_DEFAULT, V::Enumerated(state))]);
                    faults.push(vec![row(P::POLARITY, V::Enumerated(state))]);
                }
            }
            Family::MultiState => {
                for states in [0, 1, 3, 1024, 1025] {
                    faults.push(vec![row(P::NUMBER_OF_STATES, V::Unsigned(states))]);
                }
                for labels in 0..=4 {
                    let texts = (0..labels).map(|n| text(&format!("s{n}"))).collect();
                    faults.push(vec![row(P::STATE_TEXT, V::List(texts))]);
                }
                for state in [0, 1, 3, 4] {
                    faults.push(vec![row(P::RELINQUISH_DEFAULT, V::Unsigned(state))]);
                }
            }
            Family::Integer => {
                for increment in [0, u64::MAX] {
                    faults.push(vec![row(P::COV_INCREMENT, V::Unsigned(increment))]);
                }
                for (min, max) in [(1, 1), (2, 1), (i32::MIN, i32::MAX), (i32::MAX, i32::MIN)] {
                    faults.push(vec![
                        row(P::MIN_PRES_VALUE, V::Signed(min)),
                        row(P::MAX_PRES_VALUE, V::Signed(max)),
                    ]);
                }
                for default in [-7, i32::MIN] {
                    faults.push(vec![row(P::RELINQUISH_DEFAULT, V::Signed(default))]);
                }
            }
        }
        if matches!(family, Family::Analog | Family::Integer) {
            for units in [0, 65_535, 65_536, u32::MAX] {
                faults.push(vec![row(P::UNITS, V::Enumerated(units))]);
            }
        }
        for extra in faults {
            for commanded in [false, true] {
                cases.push((object_type, rows(object_type, commanded, extra.clone())));
            }
        }
    }
    let at = crate::DeviceAddress::local(&[2]).unwrap();
    let (mut refused, mut built) = (0, 0);
    for (object_type, rows) in cases {
        let record = crate::inventory::record_of(&at, LAB, oid(object_type, 1), rows);
        let point = record.to_point().unwrap_or_else(|| panic!("{record:?}"));
        let named: Vec<P> = record
            .problems
            .iter()
            .map(|problem| problem.property)
            .collect();
        match build_object(point) {
            Ok(_) => {
                built += 1;
                assert!(named.is_empty(), "the builder took it: {record:?}");
            }
            Err(error) => {
                refused += 1;
                let property = error.property.unwrap_or_else(|| panic!("{error:?}"));
                assert!(named.contains(&property), "{error:?}: {record:?}");
            }
        }
    }
    assert!(
        refused > 100 && built > 100,
        "{refused} refused, {built} built"
    );
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(LAB, 42).unwrap()
}

fn commanded<T>(relinquish_default: Option<T>) -> Access<T> {
    Access::Commandable { relinquish_default }
}

/// One point of each of the ten types, every setting within range.
fn list() -> PointList {
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    let analog = |access| {
        Analog::new(celsius, access)
            .with_cov_increment(0.5)
            .with_min_pres_value(-40.0)
            .with_max_pres_value(120.0)
    };
    let integer = |access| {
        Integer::new(EngineeringUnits::KILOWATT_HOURS, access)
            .with_cov_increment(10)
            .with_min_pres_value(-500)
            .with_max_pres_value(500)
    };
    let point =
        |object_type, name: &str, kind: PointKind| Point::new(oid(object_type, 1), name, kind);
    PointList::new(
        DeviceInfo::new(LAB),
        vec![
            point(
                ObjectType::ANALOG_INPUT,
                "ai",
                analog(Access::ReadOnly).into(),
            ),
            point(
                ObjectType::ANALOG_OUTPUT,
                "ao",
                analog(commanded(Some(12.5))).into(),
            ),
            point(
                ObjectType::ANALOG_VALUE,
                "av",
                analog(commanded(Some(21.5))).into(),
            ),
            point(
                ObjectType::BINARY_INPUT,
                "bi",
                Binary::new(Access::ReadOnly)
                    .with_polarity(Polarity::REVERSE)
                    .into(),
            ),
            point(
                ObjectType::BINARY_OUTPUT,
                "bo",
                Binary::new(commanded(Some(BinaryPV::ACTIVE)))
                    .with_polarity(Polarity::REVERSE)
                    .into(),
            ),
            point(
                ObjectType::BINARY_VALUE,
                "bv",
                Binary::new(commanded(Some(BinaryPV::ACTIVE))).into(),
            ),
            point(
                ObjectType::MULTI_STATE_INPUT,
                "msi",
                MultiState::new(3, Access::ReadOnly).into(),
            ),
            point(
                ObjectType::MULTI_STATE_OUTPUT,
                "mso",
                MultiState::new(3, commanded(Some(2))).into(),
            ),
            point(
                ObjectType::MULTI_STATE_VALUE,
                "msv",
                MultiState::new(3, commanded(Some(3))).into(),
            ),
            point(
                ObjectType::INTEGER_VALUE,
                "iv",
                integer(commanded(Some(-7))).into(),
            ),
        ],
    )
}

/// `list` built, served with each object behind an [`Answering`] that
/// gives `answers(object_type)`, and read back by the point list's
/// options over a link with `faults`.
async fn read_answering(
    faults: Faults,
    answers: impl Fn(ObjectType) -> Vec<(P, Answer)>,
) -> Inventory {
    let mut served = build_database(list(), &identity()).unwrap();
    let mut source = build_database(list(), &identity()).unwrap();
    for point in &list().points {
        let object = source.remove(&point.object).unwrap().unwrap();
        let answers = answers(point.object.object_type());
        served.add(Answering::wrap(object, answers)).unwrap();
    }
    let (client, server) = serve(faults, identity(), served).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&point_list_options())
        .await
        .unwrap();
    stop(client, server).await;
    assert!(inventory.stopped.is_none(), "{inventory:?}");
    inventory
}

/// `kind` with no setting: no COV_Increment, range, Polarity or relinquish
/// default.
fn unset(kind: &mut PointKind) {
    fn clear<T>(access: &mut Access<T>) {
        if let Access::Commandable { relinquish_default } = access {
            *relinquish_default = None;
        }
    }
    match kind {
        PointKind::Analog(analog) => {
            analog.cov_increment = None;
            analog.min_pres_value = None;
            analog.max_pres_value = None;
            clear(&mut analog.access);
        }
        PointKind::Binary(binary) => {
            binary.polarity = None;
            clear(&mut binary.access);
        }
        PointKind::MultiState(multi_state) => clear(&mut multi_state.access),
        PointKind::Integer(integer) => {
            integer.cov_increment = None;
            integer.min_pres_value = None;
            integer.max_pres_value = None;
            clear(&mut integer.access);
        }
        other => panic!("{other:?}"),
    }
}

/// Every setting a point list reads, given as a character string: each
/// record names each one in a problem, `not` its datatype, and the point
/// list is the one the same device reads with its own settings, with none
/// of them; it builds. By RPM and by ReadProperty alike.
#[tokio::test(start_paused = true)]
async fn mistyped_settings_are_problems_and_absent_from_the_points() {
    let options = point_list_options();
    for faults in [Faults::default(), refusing_rpm()] {
        let plain = read_answering(faults.clone(), |_| Vec::new()).await;
        assert!(plain.points.iter().all(|record| record.problems.is_empty()));
        let mut expected = Model::from(&plain).point_list();
        for point in &mut expected.points {
            unset(&mut point.kind);
        }

        let settings = |object_type| -> Vec<P> {
            crate::inventory::properties(object_type, &options)
                .into_iter()
                .filter(|property| {
                    [
                        P::COV_INCREMENT,
                        P::MIN_PRES_VALUE,
                        P::MAX_PRES_VALUE,
                        P::POLARITY,
                        P::RELINQUISH_DEFAULT,
                    ]
                    .contains(property)
                })
                .collect()
        };
        let wrong = |object_type| {
            settings(object_type)
                .into_iter()
                .map(|property| (property, Ok(text("wrong"))))
                .collect()
        };
        let inventory = read_answering(faults, wrong).await;
        for record in &inventory.points {
            let asked = settings(record.object.object_type());
            let problems: Vec<P> = record
                .problems
                .iter()
                .map(|problem| problem.property)
                .collect();
            assert_eq!(problems, asked, "{record:?}");
            for property in asked {
                let said = record.problem(property).unwrap().to_string();
                assert!(said.starts_with("not a"), "{said}");
            }
        }
        let model = Model::from(&inventory);
        assert!(model.entries.iter().all(Result::is_ok), "{model:?}");
        assert_eq!(model.point_list(), expected);
        build_database(model.point_list(), &identity()).unwrap();
    }
}

/// A setting the builder refuses, one on each type that has one: the
/// record keeps it and names it in a problem, the point holds it, and the
/// builder refuses that point for the property the record names. The
/// multi-state input, which reads no setting, builds. By RPM and by
/// ReadProperty alike.
#[tokio::test(start_paused = true)]
async fn refused_settings_are_problems_the_builder_agrees_with() {
    let answers = |object_type| -> Vec<(P, Answer)> {
        type O = ObjectType;
        let real = |value| Ok(PropertyValue::Real(value));
        match object_type {
            O::ANALOG_INPUT => vec![
                (P::MIN_PRES_VALUE, real(10.0)),
                (P::MAX_PRES_VALUE, real(5.0)),
            ],
            O::ANALOG_OUTPUT => vec![(P::RELINQUISH_DEFAULT, real(f32::NAN))],
            O::ANALOG_VALUE => vec![(P::COV_INCREMENT, real(-1.0))],
            O::BINARY_INPUT => vec![(P::POLARITY, Ok(PropertyValue::Enumerated(3)))],
            O::BINARY_OUTPUT => vec![(P::RELINQUISH_DEFAULT, Ok(PropertyValue::Enumerated(2)))],
            O::BINARY_VALUE => vec![(P::RELINQUISH_DEFAULT, Ok(PropertyValue::Enumerated(5)))],
            O::MULTI_STATE_OUTPUT => vec![(P::RELINQUISH_DEFAULT, Ok(PropertyValue::Unsigned(0)))],
            O::MULTI_STATE_VALUE => vec![(P::RELINQUISH_DEFAULT, Ok(PropertyValue::Unsigned(4)))],
            O::INTEGER_VALUE => vec![
                (P::MIN_PRES_VALUE, Ok(PropertyValue::Signed(9))),
                (P::MAX_PRES_VALUE, Ok(PropertyValue::Signed(1))),
            ],
            _ => Vec::new(),
        }
    };
    for faults in [Faults::default(), refusing_rpm()] {
        let inventory = read_answering(faults, answers).await;
        let model = Model::from(&inventory);
        assert_eq!(model.points().count(), 10, "{model:?}");
        for record in &inventory.points {
            if record.object.object_type() == ObjectType::DEVICE {
                continue;
            }
            let built = build_object(record.to_point().unwrap());
            match record.problems.as_slice() {
                [] => {
                    assert_eq!(record.object.object_type(), ObjectType::MULTI_STATE_INPUT);
                    assert!(built.is_ok(), "{record:?}");
                }
                [problem] => {
                    let refused = built.err().unwrap_or_else(|| panic!("{record:?}"));
                    assert_eq!(refused.property, Some(problem.property), "{record:?}");
                }
                more => panic!("{more:?}"),
            }
        }
        // The values are kept as the device gave them.
        let mso = inventory
            .point(oid(ObjectType::MULTI_STATE_OUTPUT, 1))
            .unwrap();
        assert_eq!(mso.relinquish_default.as_ref().unwrap().as_u64(), Some(0));
        let bi = inventory.point(oid(ObjectType::BINARY_INPUT, 1)).unwrap();
        assert_eq!(
            bi.problem(P::POLARITY).unwrap().to_string(),
            "neither normal nor reverse"
        );
        let polarity = model.points().find(|point| point.object == bi.object);
        let PointKind::Binary(binary) = &polarity.unwrap().kind else {
            panic!("a binary input");
        };
        assert_eq!(binary.polarity, Some(Polarity::from_raw(3)));
        assert!(build_database(model.point_list(), &identity()).is_err());
    }
}
