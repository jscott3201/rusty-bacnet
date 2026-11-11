//! BACnet engineering units as QUDT units and quantity kinds, with lookups
//! both ways (feature `qudt`).
//!
//! QUDT is the unit vocabulary the ASHRAE 223 ontology uses for a
//! quantifiable property. This module is a hand-written table of the units
//! HVAC, lighting and energy points use most, as const data: no allocation,
//! no dependency, usable without `std` or `alloc`. It grows on request.
//!
//! Every name is a QUDT *local name*, the part after the `unit:` or
//! `quantitykind:` prefix (`DEG_F`, `Temperature`), as a `&'static str`.
//!
//! # Lookups
//!
//! - [`qudt_unit`] maps an [`EngineeringUnits`](crate::enums::EngineeringUnits) value to its QUDT unit and
//!   quantity kind, or `None` when the table has no row for it.
//! - [`engineering_units_from_qudt`] maps a QUDT unit local name back, or
//!   `None` when the name is not in the table. QUDT units the table lacks, and
//!   spelling variants of the names it has, return `None`; none are guessed.
//!
//! Both are a linear scan over the table (about ninety rows), so neither
//! allocates and neither needs a sorted layout or a hash.
//!
//! # Aliases
//!
//! Two BACnet units can share a QUDT unit, so the reverse lookup is not
//! injective. The only such case today is `UNITLESS`, which both
//! [`EngineeringUnits::NO_UNITS`](crate::enums::EngineeringUnits::NO_UNITS) and [`EngineeringUnits::POWER_FACTOR`](crate::enums::EngineeringUnits::POWER_FACTOR) map
//! to (they differ in quantity kind). The reverse lookup returns
//! [`EngineeringUnits::NO_UNITS`](crate::enums::EngineeringUnits::NO_UNITS) for it: the plain reading of a
//! dimensionless unit, and the row that comes first in the table.

use crate::enums::EngineeringUnits as U;

/// `(BACnet unit, QUDT unit, QUDT quantity kind)`. For a QUDT unit that
/// more than one row names, the first row is the reverse lookup's answer.
const TABLE: &[(U, &str, &str)] = &[
    // Temperature and humidity.
    (U::DEGREES_FAHRENHEIT, "DEG_F", "Temperature"),
    (U::DEGREES_CELSIUS, "DEG_C", "Temperature"),
    (U::DEGREES_KELVIN, "K", "Temperature"),
    (
        U::PERCENT_RELATIVE_HUMIDITY,
        "PERCENT_RH",
        "RelativeHumidity",
    ),
    // Volume and mass flow.
    (U::CUBIC_FEET_PER_MINUTE, "FT3-PER-MIN", "VolumeFlowRate"),
    (U::CUBIC_FEET_PER_SECOND, "FT3-PER-SEC", "VolumeFlowRate"),
    (U::CUBIC_FEET_PER_HOUR, "FT3-PER-HR", "VolumeFlowRate"),
    (U::CUBIC_METERS_PER_SECOND, "M3-PER-SEC", "VolumeFlowRate"),
    (U::CUBIC_METERS_PER_MINUTE, "M3-PER-MIN", "VolumeFlowRate"),
    (U::CUBIC_METERS_PER_HOUR, "M3-PER-HR", "VolumeFlowRate"),
    (U::LITERS_PER_SECOND, "L-PER-SEC", "VolumeFlowRate"),
    (U::LITERS_PER_MINUTE, "L-PER-MIN", "VolumeFlowRate"),
    (U::LITERS_PER_HOUR, "L-PER-HR", "VolumeFlowRate"),
    (U::US_GALLONS_PER_MINUTE, "GAL_US-PER-MIN", "VolumeFlowRate"),
    (U::US_GALLONS_PER_HOUR, "GAL_US-PER-HR", "VolumeFlowRate"),
    (U::KILOGRAMS_PER_SECOND, "KiloGM-PER-SEC", "MassFlowRate"),
    (U::KILOGRAMS_PER_HOUR, "KiloGM-PER-HR", "MassFlowRate"),
    (U::POUNDS_MASS_PER_MINUTE, "LB-PER-MIN", "MassFlowRate"),
    (U::POUNDS_MASS_PER_HOUR, "LB-PER-HR", "MassFlowRate"),
    // Pressure.
    (U::PASCALS, "PA", "Pressure"),
    (U::HECTOPASCALS, "HectoPA", "Pressure"),
    (U::KILOPASCALS, "KiloPA", "Pressure"),
    (U::MILLIBARS, "MilliBAR", "Pressure"),
    (U::BARS, "BAR", "Pressure"),
    (U::POUNDS_FORCE_PER_SQUARE_INCH, "PSI", "Pressure"),
    (U::INCHES_OF_WATER, "IN_H2O", "Pressure"),
    (U::INCHES_OF_MERCURY, "IN_HG", "Pressure"),
    (U::MILLIMETERS_OF_MERCURY, "MilliM_HG", "Pressure"),
    // Power and energy.
    (U::MILLIWATTS, "MilliW", "Power"),
    (U::WATTS, "W", "Power"),
    (U::KILOWATTS, "KiloW", "Power"),
    (U::MEGAWATTS, "MegaW", "Power"),
    (U::BTUS_PER_HOUR, "BTU_IT-PER-HR", "Power"),
    (U::KILO_BTUS_PER_HOUR, "KiloBTU_IT-PER-HR", "Power"),
    (U::HORSEPOWER, "HP", "Power"),
    (U::TONS_REFRIGERATION, "TON_FG", "Power"),
    (U::JOULES, "J", "Energy"),
    (U::KILOJOULES, "KiloJ", "Energy"),
    (U::MEGAJOULES, "MegaJ", "Energy"),
    (U::WATT_HOURS, "W-HR", "Energy"),
    (U::KILOWATT_HOURS, "KiloW-HR", "Energy"),
    (U::MEGAWATT_HOURS, "MegaW-HR", "Energy"),
    (U::BTUS, "BTU_IT", "Energy"),
    (U::KILO_BTUS, "KiloBTU_IT", "Energy"),
    // Electrical.
    (U::VOLTS, "V", "Voltage"),
    (U::MILLIVOLTS, "MilliV", "Voltage"),
    (U::KILOVOLTS, "KiloV", "Voltage"),
    (U::AMPERES, "A", "ElectricCurrent"),
    (U::MILLIAMPERES, "MilliA", "ElectricCurrent"),
    (U::OHMS, "OHM", "Resistance"),
    (U::KILOOHMS, "KiloOHM", "Resistance"),
    (U::MEGAOHMS, "MegaOHM", "Resistance"),
    (U::VOLT_AMPERES, "V-A", "ApparentPower"),
    (U::KILOVOLT_AMPERES, "KiloV-A", "ApparentPower"),
    (U::VOLT_AMPERES_REACTIVE, "V-A_Reactive", "ReactivePower"),
    (
        U::KILOVOLT_AMPERES_REACTIVE,
        "KiloV-A_Reactive",
        "ReactivePower",
    ),
    (U::HERTZ, "HZ", "Frequency"),
    (U::KILOHERTZ, "KiloHZ", "Frequency"),
    (U::REVOLUTIONS_PER_MINUTE, "REV-PER-MIN", "AngularVelocity"),
    // Time, length, speed, area, volume and mass.
    (U::MILLISECONDS, "MilliSEC", "Time"),
    (U::SECONDS, "SEC", "Time"),
    (U::MINUTES, "MIN", "Time"),
    (U::HOURS, "HR", "Time"),
    (U::DAYS, "DAY", "Time"),
    (U::MILLIMETERS, "MilliM", "Length"),
    (U::CENTIMETERS, "CentiM", "Length"),
    (U::METERS, "M", "Length"),
    (U::INCHES, "IN", "Length"),
    (U::FEET, "FT", "Length"),
    (U::METERS_PER_SECOND, "M-PER-SEC", "Velocity"),
    (U::FEET_PER_SECOND, "FT-PER-SEC", "Velocity"),
    (U::FEET_PER_MINUTE, "FT-PER-MIN", "Velocity"),
    (U::SQUARE_METERS, "M2", "Area"),
    (U::SQUARE_FEET, "FT2", "Area"),
    (U::CUBIC_METERS, "M3", "Volume"),
    (U::CUBIC_FEET, "FT3", "Volume"),
    (U::LITERS, "L", "Volume"),
    (U::US_GALLONS, "GAL_US", "Volume"),
    (U::KILOGRAMS, "KiloGM", "Mass"),
    (U::POUNDS_MASS, "LB", "Mass"),
    // Light.
    (U::LUXES, "LUX", "Illuminance"),
    (U::FOOT_CANDLES, "FC", "Illuminance"),
    (U::LUMENS, "LM", "LuminousFlux"),
    // Ratios, concentrations and angles.
    (U::PERCENT, "PERCENT", "DimensionlessRatio"),
    (U::PARTS_PER_MILLION, "PPM", "DimensionlessRatio"),
    (U::PARTS_PER_BILLION, "PPB", "DimensionlessRatio"),
    // `UNITLESS` is shared: the first row is what the reverse lookup returns.
    (U::NO_UNITS, "UNITLESS", "Dimensionless"),
    (U::POWER_FACTOR, "UNITLESS", "PowerFactor"),
    (U::DEGREES_ANGULAR, "DEG", "PlaneAngle"),
];

/// `units` as a QUDT unit and quantity kind (local names), or `None` when
/// the table has no row for it.
pub fn qudt_unit(units: U) -> Option<(&'static str, &'static str)> {
    TABLE
        .iter()
        .find(|(bacnet, _, _)| *bacnet == units)
        .map(|&(_, unit, kind)| (unit, kind))
}

/// The BACnet unit for a QUDT unit local name (such as `"DEG_F"`), or `None`
/// when the name is not in the table. Where two BACnet units share a name,
/// the first row's unit is returned (see the module's alias notes).
pub fn engineering_units_from_qudt(unit: &str) -> Option<U> {
    TABLE
        .iter()
        .find(|(_, name, _)| *name == unit)
        .map(|&(bacnet, _, _)| bacnet)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QUDT units that more than one row names, with the BACnet unit the
    /// reverse lookup answers.
    const ACCEPTED_ALIASES: &[(&str, U)] = &[("UNITLESS", U::NO_UNITS)];

    /// Each BACnet unit has one row, and every local name is a plain
    /// prefixed-name local part (letters, digits, `_` and `-`), so it can
    /// follow `unit:` or `quantitykind:` without escaping.
    #[test]
    fn rows_are_unique_and_names_need_no_escaping() {
        for (i, (bacnet, unit, kind)) in TABLE.iter().enumerate() {
            assert!(
                TABLE[i + 1..].iter().all(|(other, _, _)| other != bacnet),
                "{bacnet} twice"
            );
            for name in [unit, kind] {
                assert!(
                    name.starts_with(|c: char| c.is_ascii_alphabetic())
                        && name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                    "{name}"
                );
            }
        }
        assert!(TABLE.len() >= 60, "{}", TABLE.len());
        assert_eq!(
            qudt_unit(U::DEGREES_FAHRENHEIT),
            Some(("DEG_F", "Temperature"))
        );
        assert_eq!(qudt_unit(U::DEGREE_DAYS_CELSIUS), None);
    }

    /// Every row's unit name reads back to a unit with the same name, and
    /// to the row's own unit unless the name is an accepted alias.
    #[test]
    fn every_row_round_trips() {
        for &(bacnet, unit, _) in TABLE {
            let back = engineering_units_from_qudt(unit).expect(unit);
            match ACCEPTED_ALIASES.iter().find(|(name, _)| *name == unit) {
                Some(&(_, canonical)) => assert_eq!(back, canonical, "{unit}"),
                None => assert_eq!(back, bacnet, "{unit}"),
            }
            assert_eq!(qudt_unit(back).map(|(name, _)| name), Some(unit));
        }
    }

    /// No two rows share a QUDT unit unless it is listed as an alias, and
    /// every alias is shared by at least two rows.
    #[test]
    fn shared_qudt_units_are_exactly_the_accepted_aliases() {
        for (i, (_, unit, _)) in TABLE.iter().enumerate() {
            let shared = TABLE[i + 1..].iter().any(|(_, other, _)| other == unit);
            let listed = ACCEPTED_ALIASES.iter().any(|(name, _)| name == unit);
            assert!(!shared || listed, "{unit} shared but not listed");
        }
        for (name, canonical) in ACCEPTED_ALIASES {
            let rows = TABLE.iter().filter(|(_, unit, _)| unit == name).count();
            assert!(rows >= 2, "{name} listed but not shared");
            assert_eq!(
                TABLE.iter().find(|(_, u, _)| u == name).unwrap().0,
                *canonical
            );
        }
    }

    /// Names outside the table, including near misses, return `None`.
    #[test]
    fn unknown_names_return_none() {
        for name in ["", "deg_f", "DEG_F ", "unit:DEG_F", "KiloGM-PER-DAY"] {
            assert_eq!(engineering_units_from_qudt(name), None, "{name:?}");
        }
        assert_eq!(
            engineering_units_from_qudt("DEG_F"),
            Some(U::DEGREES_FAHRENHEIT)
        );
    }
}
