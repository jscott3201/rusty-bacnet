// ===========================================================================
// EngineeringUnits (Clause 21) — large enum, grouped by category
// ===========================================================================

#[cfg(all(feature = "serde", not(feature = "std")))]
use alloc::string::String;

#[cfg(feature = "serde")]
use serde::Deserialize;

bacnet_enum! {
    /// BACnet engineering units (Clause 21).
    ///
    /// ASHRAE owns 0-255 and 47808-49999. Vendor units go in 256-47807 or
    /// 50000-65535, following the extension rules of Clause 23.
    pub struct EngineeringUnits(u32);

    // Acceleration
    const METERS_PER_SECOND_PER_SECOND = 166;
    // Area
    const SQUARE_METERS = 0;
    const SQUARE_CENTIMETERS = 116;
    const SQUARE_FEET = 1;
    const SQUARE_INCHES = 115;
    // Currency
    const CURRENCY1 = 105;
    const CURRENCY2 = 106;
    const CURRENCY3 = 107;
    const CURRENCY4 = 108;
    const CURRENCY5 = 109;
    const CURRENCY6 = 110;
    const CURRENCY7 = 111;
    const CURRENCY8 = 112;
    const CURRENCY9 = 113;
    const CURRENCY10 = 114;
    // Electrical
    const MILLIAMPERES = 2;
    const AMPERES = 3;
    const AMPERES_PER_METER = 167;
    const AMPERES_PER_SQUARE_METER = 168;
    const AMPERE_SQUARE_METERS = 169;
    const DECIBELS = 199;
    const DECIBELS_MILLIVOLT = 200;
    const DECIBELS_VOLT = 201;
    const FARADS = 170;
    const HENRYS = 171;
    const OHMS = 4;
    const OHM_METER_SQUARED_PER_METER = 237;
    const OHM_METERS = 172;
    const MILLIOHMS = 145;
    const KILOOHMS = 122;
    const MEGAOHMS = 123;
    const MICROSIEMENS = 190;
    const MILLISIEMENS = 202;
    const SIEMENS = 173;
    const SIEMENS_PER_METER = 174;
    const TESLAS = 175;
    const VOLTS = 5;
    const MILLIVOLTS = 124;
    const KILOVOLTS = 6;
    const MEGAVOLTS = 7;
    const VOLT_AMPERES = 8;
    const KILOVOLT_AMPERES = 9;
    const MEGAVOLT_AMPERES = 10;
    const VOLT_AMPERES_REACTIVE = 11;
    const KILOVOLT_AMPERES_REACTIVE = 12;
    const MEGAVOLT_AMPERES_REACTIVE = 13;
    const VOLTS_PER_DEGREE_KELVIN = 176;
    const VOLTS_PER_METER = 177;
    const DEGREES_PHASE = 14;
    const POWER_FACTOR = 15;
    const WEBERS = 178;
    // Energy
    const AMPERE_SECONDS = 238;
    const VOLT_AMPERE_HOURS = 239;
    const KILOVOLT_AMPERE_HOURS = 240;
    const MEGAVOLT_AMPERE_HOURS = 241;
    const VOLT_AMPERE_HOURS_REACTIVE = 242;
    const KILOVOLT_AMPERE_HOURS_REACTIVE = 243;
    const MEGAVOLT_AMPERE_HOURS_REACTIVE = 244;
    const VOLT_SQUARE_HOURS = 245;
    const AMPERE_SQUARE_HOURS = 246;
    const JOULES = 16;
    const KILOJOULES = 17;
    const KILOJOULES_PER_KILOGRAM = 125;
    const MEGAJOULES = 126;
    const WATT_HOURS = 18;
    const KILOWATT_HOURS = 19;
    const MEGAWATT_HOURS = 146;
    const WATT_HOURS_REACTIVE = 203;
    const KILOWATT_HOURS_REACTIVE = 204;
    const MEGAWATT_HOURS_REACTIVE = 205;
    const BTUS = 20;
    const KILO_BTUS = 147;
    const MEGA_BTUS = 148;
    const THERMS = 21;
    const TON_HOURS = 22;
    // Enthalpy
    const JOULES_PER_KILOGRAM_DRY_AIR = 23;
    const KILOJOULES_PER_KILOGRAM_DRY_AIR = 149;
    const MEGAJOULES_PER_KILOGRAM_DRY_AIR = 150;
    const BTUS_PER_POUND_DRY_AIR = 24;
    const BTUS_PER_POUND = 117;
    // Entropy
    const JOULES_PER_DEGREE_KELVIN = 127;
    const KILOJOULES_PER_DEGREE_KELVIN = 151;
    const MEGAJOULES_PER_DEGREE_KELVIN = 152;
    const JOULES_PER_KILOGRAM_DEGREE_KELVIN = 128;
    // Force
    const NEWTON = 153;
    // Frequency
    const CYCLES_PER_HOUR = 25;
    const CYCLES_PER_MINUTE = 26;
    const HERTZ = 27;
    const KILOHERTZ = 129;
    const MEGAHERTZ = 130;
    const PER_HOUR = 131;
    // Humidity
    const GRAMS_OF_WATER_PER_KILOGRAM_DRY_AIR = 28;
    const PERCENT_RELATIVE_HUMIDITY = 29;
    // Length
    const MICROMETERS = 194;
    const MILLIMETERS = 30;
    const CENTIMETERS = 118;
    const KILOMETERS = 193;
    const METERS = 31;
    const INCHES = 32;
    const FEET = 33;
    // Light
    const CANDELAS = 179;
    const CANDELAS_PER_SQUARE_METER = 180;
    const WATTS_PER_SQUARE_FOOT = 34;
    const WATTS_PER_SQUARE_METER = 35;
    const LUMENS = 36;
    const LUXES = 37;
    const FOOT_CANDLES = 38;
    // Mass
    const MILLIGRAMS = 196;
    const GRAMS = 195;
    const KILOGRAMS = 39;
    const POUNDS_MASS = 40;
    const TONS = 41;
    // Mass flow
    const GRAMS_PER_SECOND = 154;
    const GRAMS_PER_MINUTE = 155;
    const KILOGRAMS_PER_SECOND = 42;
    const KILOGRAMS_PER_MINUTE = 43;
    const KILOGRAMS_PER_HOUR = 44;
    const POUNDS_MASS_PER_SECOND = 119;
    const POUNDS_MASS_PER_MINUTE = 45;
    const POUNDS_MASS_PER_HOUR = 46;
    const TONS_PER_HOUR = 156;
    // Power
    const MILLIWATTS = 132;
    const WATTS = 47;
    const KILOWATTS = 48;
    const MEGAWATTS = 49;
    const BTUS_PER_HOUR = 50;
    const KILO_BTUS_PER_HOUR = 157;
    const JOULE_PER_HOURS = 247;
    const HORSEPOWER = 51;
    const TONS_REFRIGERATION = 52;
    // Pressure
    const PASCALS = 53;
    const HECTOPASCALS = 133;
    const KILOPASCALS = 54;
    const MILLIBARS = 134;
    const BARS = 55;
    const POUNDS_FORCE_PER_SQUARE_INCH = 56;
    const MILLIMETERS_OF_WATER = 206;
    const CENTIMETERS_OF_WATER = 57;
    const INCHES_OF_WATER = 58;
    const MILLIMETERS_OF_MERCURY = 59;
    const CENTIMETERS_OF_MERCURY = 60;
    const INCHES_OF_MERCURY = 61;
    // Temperature
    const DEGREES_CELSIUS = 62;
    const DEGREES_KELVIN = 63;
    const DEGREES_KELVIN_PER_HOUR = 181;
    const DEGREES_KELVIN_PER_MINUTE = 182;
    const DEGREES_FAHRENHEIT = 64;
    const DEGREE_DAYS_CELSIUS = 65;
    const DEGREE_DAYS_FAHRENHEIT = 66;
    const DELTA_DEGREES_FAHRENHEIT = 120;
    const DELTA_DEGREES_KELVIN = 121;
    // Time
    const YEARS = 67;
    const MONTHS = 68;
    const WEEKS = 69;
    const DAYS = 70;
    const HOURS = 71;
    const MINUTES = 72;
    const SECONDS = 73;
    const HUNDREDTHS_SECONDS = 158;
    const MILLISECONDS = 159;
    // Torque
    const NEWTON_METERS = 160;
    // Velocity
    const MILLIMETERS_PER_SECOND = 161;
    const MILLIMETERS_PER_MINUTE = 162;
    const METERS_PER_SECOND = 74;
    const METERS_PER_MINUTE = 163;
    const METERS_PER_HOUR = 164;
    const KILOMETERS_PER_HOUR = 75;
    const FEET_PER_SECOND = 76;
    const FEET_PER_MINUTE = 77;
    const MILES_PER_HOUR = 78;
    // Volume
    const CUBIC_FEET = 79;
    const CUBIC_METERS = 80;
    const IMPERIAL_GALLONS = 81;
    const MILLILITERS = 197;
    const LITERS = 82;
    const US_GALLONS = 83;
    // Volumetric flow
    const CUBIC_FEET_PER_SECOND = 142;
    const CUBIC_FEET_PER_MINUTE = 84;
    const MILLION_STANDARD_CUBIC_FEET_PER_MINUTE = 254;
    const CUBIC_FEET_PER_HOUR = 191;
    const CUBIC_FEET_PER_DAY = 248;
    const STANDARD_CUBIC_FEET_PER_DAY = 47808;
    const MILLION_STANDARD_CUBIC_FEET_PER_DAY = 47809;
    const THOUSAND_CUBIC_FEET_PER_DAY = 47810;
    const THOUSAND_STANDARD_CUBIC_FEET_PER_DAY = 47811;
    const POUNDS_MASS_PER_DAY = 47812;
    const CUBIC_METERS_PER_SECOND = 85;
    const CUBIC_METERS_PER_MINUTE = 165;
    const CUBIC_METERS_PER_HOUR = 135;
    const CUBIC_METERS_PER_DAY = 249;
    const IMPERIAL_GALLONS_PER_MINUTE = 86;
    const MILLILITERS_PER_SECOND = 198;
    const LITERS_PER_SECOND = 87;
    const LITERS_PER_MINUTE = 88;
    const LITERS_PER_HOUR = 136;
    const US_GALLONS_PER_MINUTE = 89;
    const US_GALLONS_PER_HOUR = 192;
    // Other
    const DEGREES_ANGULAR = 90;
    const DEGREES_CELSIUS_PER_HOUR = 91;
    const DEGREES_CELSIUS_PER_MINUTE = 92;
    const DEGREES_FAHRENHEIT_PER_HOUR = 93;
    const DEGREES_FAHRENHEIT_PER_MINUTE = 94;
    const JOULE_SECONDS = 183;
    const KILOGRAMS_PER_CUBIC_METER = 186;
    const KILOWATT_HOURS_PER_SQUARE_METER = 137;
    const KILOWATT_HOURS_PER_SQUARE_FOOT = 138;
    const WATT_HOURS_PER_CUBIC_METER = 250;
    const JOULES_PER_CUBIC_METER = 251;
    const MEGAJOULES_PER_SQUARE_METER = 139;
    const MEGAJOULES_PER_SQUARE_FOOT = 140;
    const MOLE_PERCENT = 252;
    const NO_UNITS = 95;
    const NEWTON_SECONDS = 187;
    const NEWTONS_PER_METER = 188;
    const PARTS_PER_MILLION = 96;
    const PARTS_PER_BILLION = 97;
    const PASCAL_SECONDS = 253;
    const PERCENT = 98;
    const PERCENT_OBSCURATION_PER_FOOT = 143;
    const PERCENT_OBSCURATION_PER_METER = 144;
    const PERCENT_PER_SECOND = 99;
    const PER_MINUTE = 100;
    const PER_SECOND = 101;
    const PSI_PER_DEGREE_FAHRENHEIT = 102;
    const RADIANS = 103;
    const RADIANS_PER_SECOND = 184;
    const REVOLUTIONS_PER_MINUTE = 104;
    const SQUARE_METERS_PER_NEWTON = 185;
    const WATTS_PER_METER_PER_DEGREE_KELVIN = 189;
    const WATTS_PER_SQUARE_METER_DEGREE_KELVIN = 141;
    const PER_MILLE = 207;
    const GRAMS_PER_GRAM = 208;
    const KILOGRAMS_PER_KILOGRAM = 209;
    const GRAMS_PER_KILOGRAM = 210;
    const MILLIGRAMS_PER_GRAM = 211;
    const MILLIGRAMS_PER_KILOGRAM = 212;
    const GRAMS_PER_MILLILITER = 213;
    const GRAMS_PER_LITER = 214;
    const MILLIGRAMS_PER_LITER = 215;
    const MICROGRAMS_PER_LITER = 216;
    const GRAMS_PER_CUBIC_METER = 217;
    const MILLIGRAMS_PER_CUBIC_METER = 218;
    const MICROGRAMS_PER_CUBIC_METER = 219;
    const NANOGRAMS_PER_CUBIC_METER = 220;
    const GRAMS_PER_CUBIC_CENTIMETER = 221;
    const BECQUERELS = 222;
    const KILOBECQUERELS = 223;
    const MEGABECQUERELS = 224;
    const GRAY = 225;
    const MILLIGRAY = 226;
    const MICROGRAY = 227;
    const SIEVERTS = 228;
    const MILLISIEVERTS = 229;
    const MICROSIEVERTS = 230;
    const MICROSIEVERTS_PER_HOUR = 231;
    const MILLIREMS = 47814;
    const MILLIREMS_PER_HOUR = 47815;
    const DECIBELS_A = 232;
    const NEPHELOMETRIC_TURBIDITY_UNIT = 233;
    const PH = 234;
    const GRAMS_PER_SQUARE_METER = 235;
    const MINUTES_PER_DEGREE_KELVIN = 236;
    const DEGREES_LOVIBOND = 47816;
    const ALCOHOL_BY_VOLUME = 47817;
    const INTERNATIONAL_BITTERING_UNITS = 47818;
    const EUROPEAN_BITTERNESS_UNITS = 47819;
    const DEGREES_PLATO = 47820;
    const SPECIFIC_GRAVITY = 47821;
    const EUROPEAN_BREWING_CONVENTION = 47822;
    // 47823-47981: units added after revision 22 (135-2020); ASHRAE's
    // revision 24 data types name them. Values the file skips stay unnamed.
    const PER_DAY = 47823;
    const PER_MILLISECOND = 47824;
    const YARDS = 47825;
    const MILES = 47826;
    const NAUTICAL_MILES = 47827;
    const METRIC_TONNES = 47830;
    const SHORT_TONS = 47831;
    const LONG_TONS = 47832;
    const GRAMS_PER_HOUR = 47833;
    const GRAMS_PER_DAY = 47834;
    const KILOGRAMS_PER_DAY = 47835;
    const SHORT_TONS_PER_SECOND = 47836;
    const SHORT_TONS_PER_MINUTE = 47837;
    const SHORT_TONS_PER_HOUR = 47838;
    const SHORT_TONS_PER_DAY = 47839;
    const METRIC_TONNES_PER_SECOND = 47840;
    const METRIC_TONNES_PER_MINUTE = 47841;
    const METRIC_TONNES_PER_HOUR = 47842;
    const METRIC_TONNES_PER_DAY = 47843;
    const LONG_TONS_PER_SECOND = 47844;
    const LONG_TONS_PER_MINUTE = 47845;
    const LONG_TONS_PER_HOUR = 47846;
    const LONG_TONS_PER_DAY = 47847;
    const BTUS_PER_SECOND = 47848;
    const BTUS_PER_MINUTE = 47849;
    const BTUS_PER_DAY = 47850;
    const KILO_BTUS_PER_SECOND = 47851;
    const KILO_BTUS_PER_MINUTE = 47852;
    const KILO_BTUS_PER_DAY = 47853;
    const MEGA_BTUS_PER_SECOND = 47854;
    const MEGA_BTUS_PER_MINUTE = 47855;
    const MEGA_BTUS_PER_HOUR = 47856;
    const MEGA_BTUS_PER_DAY = 47857;
    const JOULES_PER_SECOND = 47858;
    const JOULES_PER_MINUTE = 47859;
    const JOULES_PER_DAY = 47860;
    const KILOJOULES_PER_SECOND = 47861;
    const KILOJOULES_PER_MINUTE = 47862;
    const KILOJOULES_PER_HOUR = 47863;
    const KILOJOULES_PER_DAY = 47864;
    const MEGAJOULES_PER_SECOND = 47865;
    const MEGAJOULES_PER_MINUTE = 47866;
    const MEGAJOULES_PER_HOUR = 47867;
    const MEGAJOULES_PER_DAY = 47868;
    const DEGREES_CELSIUS_PER_DAY = 47869;
    const DEGREES_FAHRENHEIT_PER_DAY = 47871;
    const DELTA_DEGREES_CELSIUS = 47872;
    const MILLION_CUBIC_FEET_PER_MINUTE = 47873;
    const MILLION_CUBIC_FEET_PER_DAY = 47874;
    const IMPERIAL_GALLONS_PER_SECOND = 47875;
    const LITERS_PER_DAY = 47878;
    const US_GALLONS_PER_SECOND = 47879;
    const US_GALLONS_PER_DAY = 47880;
    const PERCENT_PER_MINUTE = 47881;
    const PERCENT_PER_HOUR = 47882;
    const PERCENT_PER_DAY = 47883;
    const PER_MILLION = 47884;
    const PER_BILLION = 47885;
    const MICROGRAMS_PER_KILOGRAM = 47888;
    const NANOGRAMS_PER_KILOGRAM = 47889;
    const MILLIGRAMS_PER_MILLILITER = 47890;
    const MICROGRAMS_PER_MILLILITER = 47891;
    const NANOGRAMS_PER_MILLILITER = 47892;
    const KILOGRAMS_PER_LITER = 47893;
    const NANOGRAMS_PER_LITER = 47894;
    const MILLIGRAMS_PER_CUBIC_CENTIMETER = 47895;
    const MICROGRAMS_PER_CUBIC_CENTIMETER = 47896;
    const NANOGRAMS_PER_CUBIC_CENTIMETER = 47897;
    const BTU_PER_HOUR_PER_WATT = 47898;
    const BTU_PER_WATT_HOUR_SEASONAL = 47899;
    const COEFFICIENT_OF_PERFORMANCE = 47900;
    const COEFFICIENT_OF_PERFORMANCE_SEASONAL = 47901;
    const KILOWATT_PER_TON_REFRIGERATION = 47902;
    const LUMENS_PER_WATT = 47903;
    const POUND_FORCE_FEET = 47904;
    const POUND_FORCE_INCHES = 47905;
    const OUNCE_FORCE_INCHES = 47906;
    const POUNDS_FORCE_PER_SQUARE_INCH_ABSOLUTE = 47907;
    const POUNDS_FORCE_PER_SQUARE_INCH_GAUGE = 47908;
    const MICROSIEMENS_PER_CENTIMETER = 47909;
    // 47910 stays unnamed: the revision 24 data file gives it two names, and
    // we name it once the right one is confirmed (47918 stays unnamed too).
    const MILLISIEMENS_PER_METER = 47911;
    const MILLIONS_OF_US_GALLONS = 47912;
    const MILLIONS_OF_IMPERIAL_GALLONS = 47913;
    const MILLILITERS_PER_MINUTE = 47914;
    const MILS_PER_YEAR = 47915;
    const MILLIMETERS_PER_YEAR = 47916;
    const PULSES_PER_MINUTE = 47917;
    const REACTIVE_ENERGY_PULSE_VALUE = 47919;
    const APPARENT_ENERGY_PULSE_VALUE = 47920;
    const VOLT_SQUARED_HOUR_PULSE_VALUE = 47921;
    const AMPERE_SQUARED_HOUR_PULSE_VALUE = 47922;
    const CUBIC_METER_PULSE_VALUE = 47923;
    const GIGAWATTS = 47924;
    const BITS_PER_SECOND = 47929;
    const KILOBITS_PER_SECOND = 47930;
    const MEGABITS_PER_SECOND = 47931;
    const GIGABITS_PER_SECOND = 47932;
    const BYTES_PER_SECOND = 47933;
    const KILOBYTES_PER_SECOND = 47934;
    const MEGABYTES_PER_SECOND = 47935;
    const GIGABYTES_PER_SECOND = 47936;
    const VOLUME1 = 47937;
    const VOLUME2 = 47938;
    const VOLUME3 = 47939;
    const VOLUME4 = 47940;
    const VOLUME5 = 47941;
    const VOLUME6 = 47942;
    const VOLUME7 = 47943;
    const VOLUME8 = 47944;
    const VOLUME9 = 47945;
    const VOLUME10 = 47946;
    const VOLUMETRIC_FLOW1 = 47947;
    const VOLUMETRIC_FLOW2 = 47948;
    const VOLUMETRIC_FLOW3 = 47949;
    const VOLUMETRIC_FLOW4 = 47950;
    const VOLUMETRIC_FLOW5 = 47951;
    const VOLUMETRIC_FLOW6 = 47952;
    const VOLUMETRIC_FLOW7 = 47953;
    const VOLUMETRIC_FLOW8 = 47954;
    const VOLUMETRIC_FLOW9 = 47955;
    const VOLUMETRIC_FLOW10 = 47956;
    const SITE_UNIT1 = 47958;
    const SITE_UNIT2 = 47959;
    const SITE_UNIT3 = 47960;
    const SITE_UNIT4 = 47961;
    const SITE_UNIT5 = 47962;
    const SITE_UNIT6 = 47963;
    const SITE_UNIT7 = 47964;
    const SITE_UNIT8 = 47965;
    const SITE_UNIT9 = 47966;
    const SITE_UNIT10 = 47967;
    const PARTICLES_PER_CUBIC_FOOT = 47968;
    const PARTICLES_PER_CUBIC_METER = 47969;
    const PICOCURIES_PER_LITER = 47970;
    const BECQUERELS_PER_CUBIC_METER = 47971;
    const GRAINS_OF_WATER_PER_POUND_DRY_AIR = 47972;
    const DEGREE_HOURS_CELSIUS = 47973;
    const DEGREE_HOURS_FAHRENHEIT = 47974;
    const DEGREE_MINUTES_CELSIUS = 47975;
    const DEGREE_MINUTES_FAHRENHEIT = 47976;
    const DEGREE_SECONDS_CELSIUS = 47977;
    const DEGREE_SECONDS_FAHRENHEIT = 47978;
    const MICROSECONDS = 47979;
    const NANOSECONDS = 47980;
    const PICOSECONDS = 47981;
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for EngineeringUnits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string: String = Deserialize::deserialize(deserializer)?;
        string.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::EngineeringUnits;

    fn parse(input: &str) -> EngineeringUnits {
        serde_json::from_str(&format!("\"{input}\"")).expect("deserializes")
    }

    #[test]
    fn accepts_every_supported_case_style() {
        for input in [
            "degreescelsius",
            "DEGREESCELSIUS",
            "DegreesCelsius",
            "degreesCelsius",
            "degrees_celsius",
            "DEGREES_CELSIUS",
            "degrees-celsius",
            "DEGREES-CELSIUS",
        ] {
            assert_eq!(
                parse(input),
                EngineeringUnits::DEGREES_CELSIUS,
                "input: {input}"
            );
        }
    }

    #[test]
    fn covers_the_full_standard_range() {
        assert_eq!(parse("square_meters"), EngineeringUnits::SQUARE_METERS);
        assert_eq!(
            parse("european-brewing-convention"),
            EngineeringUnits::EUROPEAN_BREWING_CONVENTION
        );
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(serde_json::from_str::<EngineeringUnits>("\"not-a-unit\"").is_err());
    }

    #[test]
    fn rejects_non_string_input() {
        assert!(serde_json::from_str::<EngineeringUnits>("126").is_err());
    }
}
