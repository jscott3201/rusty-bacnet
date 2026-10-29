//! Unit symbols for the places that show a value compactly: the watch
//! list's Value column and the browser's Present_Value.
//!
//! The table is short on purpose: the units a technician meets most, each
//! with the symbol a building-automation front end shows. Any other unit
//! falls back to its kebab-case name (`square-meters`).
//!
//! Every symbol is printable ASCII except `°`, which takes one column in
//! `unicode-width` (what ratatui lays tables out with), so a value with a
//! symbol never shifts the columns after it. `°` is East Asian Ambiguous,
//! so a terminal set to draw ambiguous characters two columns wide would
//! show it wider; the value is the last thing on a browser line, and the
//! watch table truncates to its column, so even there nothing misaligns.
//! Other non-ASCII symbols (`Ω`, `µ`, `³`, `Δ`) are left out to keep that
//! risk to one character; those units keep their names.

use bacnet_types::enums::EngineeringUnits as U;

use crate::label::kebab;

/// The symbols, by unit.
const SYMBOLS: [(U, &str); 44] = [
    (U::DEGREES_FAHRENHEIT, "°F"),
    (U::DEGREES_CELSIUS, "°C"),
    (U::DEGREES_KELVIN, "K"),
    (U::PERCENT, "%"),
    (U::PERCENT_RELATIVE_HUMIDITY, "%RH"),
    (U::PARTS_PER_MILLION, "ppm"),
    (U::PASCALS, "Pa"),
    (U::KILOPASCALS, "kPa"),
    (U::BARS, "bar"),
    (U::POUNDS_FORCE_PER_SQUARE_INCH, "psi"),
    (U::INCHES_OF_WATER, "inH2O"),
    (U::WATTS, "W"),
    (U::KILOWATTS, "kW"),
    (U::MEGAWATTS, "MW"),
    (U::WATT_HOURS, "Wh"),
    (U::KILOWATT_HOURS, "kWh"),
    (U::MEGAWATT_HOURS, "MWh"),
    (U::VOLT_AMPERES, "VA"),
    (U::KILOVOLT_AMPERES, "kVA"),
    (U::VOLTS, "V"),
    (U::MILLIVOLTS, "mV"),
    (U::KILOVOLTS, "kV"),
    (U::AMPERES, "A"),
    (U::MILLIAMPERES, "mA"),
    (U::HERTZ, "Hz"),
    (U::KILOHERTZ, "kHz"),
    (U::REVOLUTIONS_PER_MINUTE, "rpm"),
    (U::MILLISECONDS, "ms"),
    (U::SECONDS, "s"),
    (U::MINUTES, "min"),
    (U::HOURS, "h"),
    (U::DAYS, "d"),
    (U::LITERS_PER_SECOND, "L/s"),
    (U::LITERS_PER_MINUTE, "L/min"),
    (U::CUBIC_FEET_PER_MINUTE, "cfm"),
    (U::US_GALLONS_PER_MINUTE, "gpm"),
    (U::METERS_PER_SECOND, "m/s"),
    (U::FEET_PER_MINUTE, "fpm"),
    (U::MILLIMETERS, "mm"),
    (U::METERS, "m"),
    (U::INCHES, "in"),
    (U::FEET, "ft"),
    (U::KILOGRAMS, "kg"),
    (U::LUXES, "lx"),
];

/// `units` as the compact places show it: its symbol, or its kebab-case
/// name when the table has none.
pub(super) fn unit_symbol(units: U) -> String {
    SYMBOLS
        .iter()
        .find(|(unit, _)| *unit == units)
        .map_or_else(|| kebab(units), |(_, symbol)| (*symbol).to_string())
}
