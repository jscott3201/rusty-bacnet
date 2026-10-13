//! Configuration and validation tests.

use super::{ConfigError, MstpCoreConfig};
use bacnet_time::Duration;

#[test]
fn defaults_are_standard_safe() {
    let config = MstpCoreConfig::default();
    assert_eq!(config, MstpCoreConfig::STANDARD);
    assert_eq!(config.baud_rate, 9_600);
    assert_eq!(
        (super::N_POLL, super::N_RETRY_TOKEN, super::N_MIN_OCTETS),
        (50, 1, 4)
    );
    assert_eq!(
        (super::T_NO_TOKEN, super::T_SLOT),
        (Duration::from_millis(500), Duration::from_millis(10))
    );
    assert!(config.validate().is_ok());
    assert_eq!(MstpCoreConfig::for_baud_rate(9_600), Ok(config));
}

/// `for_baud_rate` and `validate` are `const`: a configuration can be
/// built and checked at compile time.
#[test]
fn a_configuration_checks_at_compile_time() {
    const FAST: MstpCoreConfig = match MstpCoreConfig::for_baud_rate(115_200) {
        Ok(config) => config,
        Err(_) => panic!("115200 baud is valid"),
    };
    const TOO_SLOW: Result<MstpCoreConfig, ConfigError> = MstpCoreConfig::for_baud_rate(2_666);
    const NONE: Result<MstpCoreConfig, ConfigError> = MstpCoreConfig::for_baud_rate(0);
    assert_eq!(FAST.baud_rate, 115_200);
    assert!(matches!(
        TOO_SLOW,
        Err(ConfigError::TurnaroundExceedsUsageDelay { .. })
    ));
    assert_eq!(NONE, Err(ConfigError::ZeroBaudRate));
}

#[test]
fn validation_rejects_zero_baud_and_bad_addresses() {
    let config = MstpCoreConfig {
        baud_rate: 0,
        ..MstpCoreConfig::default()
    };
    assert_eq!(config.validate(), Err(ConfigError::ZeroBaudRate));

    let config = MstpCoreConfig {
        max_master: 127,
        station: 128,
        ..MstpCoreConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(ConfigError::StationExceedsMaxMaster { .. })
    ));
}

#[test]
fn validation_enforces_bit_time_edges_without_overflow() {
    let config = MstpCoreConfig {
        t_frame_abort: Duration::from_millis(1),
        ..MstpCoreConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(ConfigError::FrameAbortOutOfRange { .. })
    ));

    let config = MstpCoreConfig {
        baud_rate: 1,
        ..MstpCoreConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(ConfigError::FrameAbortOutOfRange { .. })
    ));
}

/// The 32-bit bit-time arithmetic gives exactly what 64-bit division
/// gives, for every bit count it is used for, at every baud rate.
#[test]
fn bit_times_match_64_bit_division() {
    for baud in [
        1,
        7,
        9_600,
        19_200,
        38_400,
        57_600,
        76_800,
        115_200,
        1_000_003,
        u32::MAX,
    ] {
        for bits in [10u32, 15, 20, 40, 60] {
            let numerator = u64::from(bits) * 1_000_000;
            let baud64 = u64::from(baud);
            assert_eq!(
                super::ceil_bit_times(baud, bits),
                Some(Duration::from_micros(numerator.div_ceil(baud64))),
                "ceil {bits} at {baud}"
            );
            assert_eq!(
                super::floor_bit_times(baud, bits),
                Some(Duration::from_micros(numerator / baud64)),
                "floor {bits} at {baud}"
            );
        }
    }
    assert_eq!(super::ceil_bit_times(0, 10), None);
    assert_eq!(super::floor_bit_times(0, 10), None);
}

#[test]
fn baud_constructor_recomputes_bit_time_fields() {
    let config = MstpCoreConfig::for_baud_rate(19_200).expect("valid baud");
    assert!(config.validate().is_ok());
    assert!(config.t_turnaround.as_micros() >= 40_000_000 / 19_200);
}

#[test]
fn validation_rejects_infeasible_response_and_ratio_policies() {
    let config = MstpCoreConfig {
        t_turnaround: Duration::from_millis(16),
        ..MstpCoreConfig::default()
    };
    assert!(matches!(
        config.validate(),
        Err(ConfigError::TurnaroundExceedsUsageDelay { .. })
    ));

    let config = MstpCoreConfig {
        multiple_frame_ratio: 101,
        ..MstpCoreConfig::default()
    };
    assert_eq!(
        config.validate(),
        Err(ConfigError::MultipleFrameRatioOutOfRange { value: 101 })
    );
}

/// The watchdog must outlast the longest frame plus T_postdrive and the
/// 1 ms completion margin, with extended frames on or off. At 9600 baud
/// one second clears a standard frame (511 octets, 0.53 s) but not an
/// extended one (1516 octets, 1.58 s): `with_extended_frames` raises it
/// to that frame's time plus one second, and leaves a longer one alone.
#[test]
fn the_watchdog_must_outlast_the_longest_frame() {
    let standard = MstpCoreConfig::default();
    assert!(!standard.extended_frames);
    assert_eq!(standard.transmit_watchdog, Duration::from_secs(1));
    assert_eq!(
        standard.longest_frame_time(),
        Duration::from_micros(511 * 1_042)
    );
    // T_postdrive at 9600 baud is 15 bit times, 1562 us.
    assert_eq!(
        standard.minimum_transmit_watchdog(),
        Duration::from_micros(511 * 1_042 + 1_562 + 1_000)
    );

    let flagged = MstpCoreConfig {
        extended_frames: true,
        ..MstpCoreConfig::default()
    };
    let longest = Duration::from_micros(1_516 * 1_042);
    let minimum = longest + Duration::from_micros(1_562 + 1_000);
    assert_eq!(flagged.longest_frame_time(), longest);
    assert_eq!(flagged.minimum_transmit_watchdog(), minimum);
    assert_eq!(
        flagged.validate(),
        Err(ConfigError::TransmitWatchdogTooShort {
            value: Duration::from_secs(1),
            minimum,
        })
    );
    // Exactly at the boundary: one microsecond short is refused, the
    // floor itself accepted.
    let short = MstpCoreConfig {
        transmit_watchdog: minimum - Duration::from_micros(1),
        ..flagged
    };
    assert_eq!(
        short.validate(),
        Err(ConfigError::TransmitWatchdogTooShort {
            value: minimum - Duration::from_micros(1),
            minimum,
        })
    );
    let at_floor = MstpCoreConfig {
        transmit_watchdog: minimum,
        ..flagged
    };
    assert_eq!(at_floor.validate(), Ok(()));

    let extended = MstpCoreConfig::default().with_extended_frames();
    assert!(extended.extended_frames);
    assert_eq!(extended.transmit_watchdog, longest + Duration::from_secs(1));
    assert_eq!(extended.validate(), Ok(()));
    let patient = MstpCoreConfig {
        transmit_watchdog: Duration::from_secs(9),
        ..MstpCoreConfig::default()
    }
    .with_extended_frames();
    assert_eq!(patient.transmit_watchdog, Duration::from_secs(9));
    // Fast enough for one second to clear the extended floor: kept.
    let fast = MstpCoreConfig::for_baud_rate(115_200)
        .unwrap()
        .with_extended_frames();
    assert_eq!(fast.transmit_watchdog, Duration::from_secs(1));
    assert_eq!(fast.validate(), Ok(()));
}

/// Below about 5.1 kbaud the longest standard frame alone outlasts one
/// second (511 octets take 1.06 s at 4800 baud), so the rule applies
/// without extended frames too. The constructors then default to that
/// frame's time plus one second, and a hand-built one-second watchdog
/// is refused. At the standard rates nothing changes.
#[test]
fn low_baud_rates_get_a_watchdog_that_outlasts_a_standard_frame() {
    let slow = MstpCoreConfig::for_baud_rate(4_800).unwrap();
    // One octet takes 2084 us, T_postdrive 3125 us.
    let longest = Duration::from_micros(511 * 2_084);
    assert_eq!(slow.longest_frame_time(), longest);
    assert_eq!(slow.transmit_watchdog, longest + Duration::from_secs(1));
    assert!(slow.transmit_watchdog >= Duration::from_millis(1_070));
    assert_eq!(slow.validate(), Ok(()));

    let hand_built = MstpCoreConfig {
        transmit_watchdog: Duration::from_secs(1),
        ..slow
    };
    assert_eq!(
        hand_built.validate(),
        Err(ConfigError::TransmitWatchdogTooShort {
            value: Duration::from_secs(1),
            minimum: longest + Duration::from_micros(3_125 + 1_000),
        })
    );
    // The lowest rate validation takes still gets a valid default.
    assert!(MstpCoreConfig::for_baud_rate(2_667).is_ok());
    for baud in [9_600, 19_200, 38_400, 57_600, 76_800, 115_200] {
        let config = MstpCoreConfig::for_baud_rate(baud).unwrap();
        assert_eq!(config.transmit_watchdog, Duration::from_secs(1), "{baud}");
    }
}

/// The longest T_frame_abort, T_reply_timeout and T_usage_timeout
/// Clause 9.5.3 allows, which a host transport uses, validate together
/// at every standard baud rate; a microsecond more of any is refused.
/// The baud-rate constructors use the shortest timeouts, and a
/// microsecond less is refused.
#[test]
fn the_longest_permitted_timeouts_validate() {
    use super::{
        MAX_FRAME_ABORT, MAX_REPLY_TIMEOUT, MAX_USAGE_TIMEOUT, MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT,
    };
    assert_eq!(MAX_FRAME_ABORT, Duration::from_millis(100));
    assert_eq!(MAX_REPLY_TIMEOUT, Duration::from_millis(300));
    assert_eq!(MAX_USAGE_TIMEOUT, Duration::from_millis(35));
    assert_eq!(MIN_REPLY_TIMEOUT, Duration::from_millis(255));
    assert_eq!(MIN_USAGE_TIMEOUT, Duration::from_millis(20));
    let standard = MstpCoreConfig::STANDARD;
    assert_eq!(
        (standard.t_reply_timeout, standard.t_usage_timeout),
        (MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT)
    );
    let less = Duration::from_micros(1);
    for under in [
        MstpCoreConfig {
            t_reply_timeout: MIN_REPLY_TIMEOUT - less,
            ..standard
        },
        MstpCoreConfig {
            t_usage_timeout: MIN_USAGE_TIMEOUT - less,
            ..standard
        },
    ] {
        assert!(under.validate().is_err());
    }
    for baud in [9_600, 19_200, 38_400, 57_600, 76_800, 115_200] {
        let host = MstpCoreConfig {
            t_frame_abort: MAX_FRAME_ABORT,
            t_reply_timeout: MAX_REPLY_TIMEOUT,
            t_usage_timeout: MAX_USAGE_TIMEOUT,
            ..MstpCoreConfig::for_baud_rate(baud).unwrap()
        };
        assert_eq!(host.validate(), Ok(()), "{baud}");
        let more = Duration::from_micros(1);
        for over in [
            MstpCoreConfig {
                t_frame_abort: MAX_FRAME_ABORT + more,
                ..host
            },
            MstpCoreConfig {
                t_reply_timeout: MAX_REPLY_TIMEOUT + more,
                ..host
            },
            MstpCoreConfig {
                t_usage_timeout: MAX_USAGE_TIMEOUT + more,
                ..host
            },
        ] {
            assert!(over.validate().is_err(), "{baud}");
        }
    }
}

#[test]
fn reply_margin_must_leave_part_of_the_reply_delay() {
    let config = MstpCoreConfig::default();
    assert_eq!(config.reply_transmit_margin, Duration::from_millis(25));

    let config = MstpCoreConfig {
        reply_transmit_margin: Duration::from_millis(250),
        ..MstpCoreConfig::default()
    };
    assert_eq!(
        config.validate(),
        Err(ConfigError::ReplyMarginTooLong {
            margin: Duration::from_millis(250),
            reply_delay: Duration::from_millis(250),
        })
    );

    let config = MstpCoreConfig {
        t_reply_delay: Duration::from_millis(100),
        reply_transmit_margin: Duration::from_micros(99_999),
        ..MstpCoreConfig::default()
    };
    assert_eq!(config.validate(), Ok(()));
}
