// ===========================================================================
// EngineeringUnits (Clause 21) — large enum, grouped by category
// ===========================================================================

bacnet_enum! {
    /// BACnet engineering units (Clause 21).
    ///
    /// Values 0-255 and 47808-49999 are reserved for ASHRAE;
    /// 256-47807 and 50000-65535 may be used by vendors (Clause 23).
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
    const KILOHMS = 122;
    const MEGOHMS = 123;
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
}
