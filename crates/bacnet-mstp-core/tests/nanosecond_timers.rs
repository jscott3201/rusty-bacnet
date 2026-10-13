//! The move from microseconds to `bacnet-time` nanoseconds (#1681) left every
//! timer where it was. The values below are what the core computed when it
//! counted microseconds, printed from that build; each must now be exactly
//! 1000 times as many nanoseconds.

use bacnet_mstp_core::{ConfigError, Duration, MstpCoreConfig, LATE_DEADLINE, T_NO_TOKEN, T_SLOT};

/// One baud rate's values from the microsecond core, all in microseconds.
struct Golden {
    baud: u32,
    /// `octet_time()`.
    octet: u64,
    /// `longest_frame_time()`, standard then extended.
    longest: [u64; 2],
    /// `minimum_transmit_watchdog()`, standard then extended, of the default
    /// configuration with this baud rate set by hand.
    hand_minimum_watchdog: [u64; 2],
    /// Validation's bounds: T_frame_abort's minimum, T_frame_gap's and
    /// T_postdrive's maximums, T_turnaround's minimum.
    bounds: [u64; 4],
    /// `for_baud_rate`: T_frame_abort, T_frame_gap, T_postdrive,
    /// T_turnaround, the transmit watchdog, the watchdog after
    /// `with_extended_frames`, and that configuration's minimum watchdog.
    /// `None` where the rate is too slow for T_turnaround to fit within
    /// T_usage_delay.
    built: Option<[u64; 7]>,
}

/// The standard rates, 9600 to 115200, and slower ones down to where the
/// longest standard frame outlasts the default one-second watchdog (about
/// 5.1 kbaud) and past where the baud-rate constructor refuses the rate
/// (below 2667).
#[rustfmt::skip]
const GOLDEN: [Golden; 17] = [
    Golden { baud: 1200, octet: 8334, longest: [4258674, 12634344], hand_minimum_watchdog: [4261236, 12636906], bounds: [50000, 16666, 12500, 33334], built: None },
    Golden { baud: 2400, octet: 4167, longest: [2129337, 6317172], hand_minimum_watchdog: [2131899, 6319734], bounds: [25000, 8333, 6250, 16667], built: None },
    Golden { baud: 2666, octet: 3751, longest: [1916761, 5686516], hand_minimum_watchdog: [1919323, 5689078], bounds: [22506, 7501, 5626, 15004], built: None },
    Golden { baud: 2667, octet: 3750, longest: [1916250, 5685000], hand_minimum_watchdog: [1918812, 5687562], bounds: [22498, 7499, 5624, 14999], built: Some([22498, 7499, 5624, 14999, 2916250, 6685000, 5691624]) },
    Golden { baud: 3600, octet: 2778, longest: [1419558, 4211448], hand_minimum_watchdog: [1422120, 4214010], bounds: [16667, 5555, 4166, 11112], built: Some([16667, 5555, 4166, 11112, 2419558, 5211448, 4216614]) },
    Golden { baud: 4800, octet: 2084, longest: [1064924, 3159344], hand_minimum_watchdog: [1067486, 3161906], bounds: [12500, 4166, 3125, 8334], built: Some([12500, 4166, 3125, 8334, 2064924, 4159344, 3163469]) },
    Golden { baud: 5100, octet: 1961, longest: [1002071, 2972876], hand_minimum_watchdog: [1004633, 2975438], bounds: [11765, 3921, 2941, 7844], built: Some([11765, 3921, 2941, 7844, 2002071, 3972876, 2976817]) },
    Golden { baud: 5200, octet: 1924, longest: [983164, 2916784], hand_minimum_watchdog: [985726, 2919346], bounds: [11539, 3846, 2884, 7693], built: Some([11539, 3846, 2884, 7693, 1000000, 3916784, 2920668]) },
    Golden { baud: 7200, octet: 1389, longest: [709779, 2105724], hand_minimum_watchdog: [712341, 2108286], bounds: [8334, 2777, 2083, 5556], built: Some([8334, 2777, 2083, 5556, 1000000, 3105724, 2108807]) },
    Golden { baud: 9600, octet: 1042, longest: [532462, 1579672], hand_minimum_watchdog: [535024, 1582234], bounds: [6250, 2083, 1562, 4167], built: Some([6250, 2083, 1562, 4167, 1000000, 2579672, 1582234]) },
    Golden { baud: 14400, octet: 695, longest: [355145, 1053620], hand_minimum_watchdog: [357707, 1056182], bounds: [4167, 1388, 1041, 2778], built: Some([4167, 1388, 1041, 2778, 1000000, 2053620, 1055661]) },
    Golden { baud: 19200, octet: 521, longest: [266231, 789836], hand_minimum_watchdog: [268793, 792398], bounds: [3125, 1041, 781, 2084], built: Some([3125, 1041, 781, 2084, 1000000, 1000000, 791617]) },
    Golden { baud: 28800, octet: 348, longest: [177828, 527568], hand_minimum_watchdog: [180390, 530130], bounds: [2084, 694, 520, 1389], built: Some([2084, 694, 520, 1389, 1000000, 1000000, 529088]) },
    Golden { baud: 38400, octet: 261, longest: [133371, 395676], hand_minimum_watchdog: [135933, 398238], bounds: [1563, 520, 390, 1042], built: Some([1563, 520, 390, 1042, 1000000, 1000000, 397066]) },
    Golden { baud: 57600, octet: 174, longest: [88914, 263784], hand_minimum_watchdog: [91476, 266346], bounds: [1042, 347, 260, 695], built: Some([1042, 347, 260, 695, 1000000, 1000000, 265044]) },
    Golden { baud: 76800, octet: 131, longest: [66941, 198596], hand_minimum_watchdog: [69503, 201158], bounds: [782, 260, 195, 521], built: Some([782, 260, 195, 521, 1000000, 1000000, 199791]) },
    Golden { baud: 115200, octet: 87, longest: [44457, 131892], hand_minimum_watchdog: [47019, 134454], bounds: [521, 173, 130, 348], built: Some([521, 173, 130, 348, 1000000, 1000000, 133022]) },
];

/// `micros` microseconds as the nanoseconds the core now holds.
fn ns(micros: u64) -> u64 {
    micros * 1_000
}

/// `config` with `edit` applied: `MstpCoreConfig` is `#[non_exhaustive]`,
/// so a changed copy is built by field writes.
fn with(mut config: MstpCoreConfig, edit: impl FnOnce(&mut MstpCoreConfig)) -> MstpCoreConfig {
    edit(&mut config);
    config
}

/// Validation's four bit-time bounds at `baud`, read from the errors it
/// returns for values just outside them, as the microsecond core's were.
fn bounds(baud: u32) -> [Duration; 4] {
    let base = with(MstpCoreConfig::STANDARD, |c| c.baud_rate = baud);
    let abort = match with(base, |c| c.t_frame_abort = Duration::ZERO).validate() {
        Err(ConfigError::FrameAbortOutOfRange { minimum, .. }) => minimum,
        other => panic!("{baud}: T_frame_abort {other:?}"),
    };
    let base = with(base, |c| c.t_frame_abort = Duration::from_millis(100));
    let gap = match with(base, |c| c.t_frame_gap = Duration::MAX).validate() {
        Err(ConfigError::FrameGapTooLong { maximum, .. }) => maximum,
        other => panic!("{baud}: T_frame_gap {other:?}"),
    };
    let base = with(base, |c| c.t_frame_gap = Duration::ZERO);
    let postdrive = match with(base, |c| c.t_postdrive = Duration::MAX).validate() {
        Err(ConfigError::PostdriveTooLong { maximum, .. }) => maximum,
        other => panic!("{baud}: T_postdrive {other:?}"),
    };
    let base = with(base, |c| c.t_postdrive = Duration::ZERO);
    let turnaround = match with(base, |c| c.t_turnaround = Duration::ZERO).validate() {
        Err(ConfigError::TurnaroundTooShort { minimum, .. }) => minimum,
        other => panic!("{baud}: T_turnaround {other:?}"),
    };
    [abort, gap, postdrive, turnaround]
}

#[test]
fn bit_times_are_the_microsecond_values_at_every_rate() {
    for golden in &GOLDEN {
        let baud = golden.baud;
        let hand = with(MstpCoreConfig::STANDARD, |c| c.baud_rate = baud);
        let extended = hand.with_extended_frames();
        assert_eq!(hand.octet_time().as_nanos(), ns(golden.octet), "{baud}");
        assert_eq!(
            [
                hand.longest_frame_time().as_nanos(),
                extended.longest_frame_time().as_nanos()
            ],
            golden.longest.map(ns),
            "{baud}"
        );
        assert_eq!(
            [
                hand.minimum_transmit_watchdog().as_nanos(),
                extended.minimum_transmit_watchdog().as_nanos()
            ],
            golden.hand_minimum_watchdog.map(ns),
            "{baud}"
        );
        assert_eq!(
            bounds(baud).map(Duration::as_nanos),
            golden.bounds.map(ns),
            "{baud}"
        );
        let built = MstpCoreConfig::for_baud_rate(baud).ok().map(|config| {
            let extended = config.with_extended_frames();
            [
                config.t_frame_abort,
                config.t_frame_gap,
                config.t_postdrive,
                config.t_turnaround,
                config.transmit_watchdog,
                extended.transmit_watchdog,
                extended.minimum_transmit_watchdog(),
            ]
            .map(Duration::as_nanos)
        });
        assert_eq!(built, golden.built.map(|values| values.map(ns)), "{baud}");
    }
}

/// The rates the constructor refuses are refused for the same reason, with
/// the same values.
#[test]
fn slow_rates_are_refused_as_before() {
    for (baud, turnaround) in [(1_200, 33_334), (2_400, 16_667), (2_666, 15_004)] {
        assert_eq!(
            MstpCoreConfig::for_baud_rate(baud),
            Err(ConfigError::TurnaroundExceedsUsageDelay {
                turnaround: Duration::from_nanos(ns(turnaround)),
                usage_delay: Duration::from_nanos(ns(15_000)),
            }),
            "{baud}"
        );
    }
}

/// The fixed Clause 9 timers and the local defaults, and the limits
/// validation holds them to.
#[test]
fn fixed_timers_keep_their_values() {
    let config = MstpCoreConfig::STANDARD;
    for (name, value, micros) in [
        ("T_no_token", T_NO_TOKEN, 500_000),
        ("T_reply_delay", config.t_reply_delay, 250_000),
        ("T_reply_timeout", config.t_reply_timeout, 255_000),
        ("T_slot", T_SLOT, 10_000),
        ("T_usage_delay", config.t_usage_delay, 15_000),
        ("T_usage_timeout", config.t_usage_timeout, 20_000),
        ("reply margin", config.reply_transmit_margin, 25_000),
        ("queue age", config.queue_max_age, 5_000_000),
        ("watchdog", config.transmit_watchdog, 1_000_000),
        ("late deadline", LATE_DEADLINE, 1_000),
    ] {
        assert_eq!(value.as_nanos(), ns(micros), "{name}");
    }

    let check = |config: MstpCoreConfig| config.validate();
    let micros = |value: u64| Duration::from_nanos(ns(value));
    assert!(matches!(
        check(with(config, |c| c.t_frame_abort = micros(100_001))),
        Err(ConfigError::FrameAbortOutOfRange { maximum, .. }) if maximum == micros(100_000)
    ));
    assert!(matches!(
        check(with(config, |c| c.t_reply_delay = micros(250_001))),
        Err(ConfigError::ReplyDelayTooLong { maximum, .. }) if maximum == micros(250_000)
    ));
    assert!(matches!(
        check(with(config, |c| c.t_reply_timeout = micros(300_001))),
        Err(ConfigError::ReplyTimeoutOutOfRange { minimum, maximum, .. })
            if minimum == micros(255_000) && maximum == micros(300_000)
    ));
    assert!(matches!(
        check(with(config, |c| { c.t_usage_delay = micros(15_001); c.t_turnaround = micros(4_167); })),
        Err(ConfigError::UsageDelayTooLong { maximum, .. }) if maximum == micros(15_000)
    ));
    assert!(matches!(
        check(with(config, |c| c.t_usage_timeout = micros(35_001))),
        Err(ConfigError::UsageTimeoutOutOfRange { minimum, maximum, .. })
            if minimum == micros(20_000) && maximum == micros(35_000)
    ));
}
