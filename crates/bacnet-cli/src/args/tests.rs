use super::*;
use bacnet_types::primitives::{Date, Time};
use clap::error::ErrorKind;

// The derived parser's debug-build frames nearly fill a 1 MiB test thread,
// so parse where `main` does: on a thread with a larger stack (#953).
fn parse<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
    crate::parse_on_large_stack(move || {
        use clap::{CommandFactory, FromArgMatches};
        let mut matches = Cli::command().try_get_matches_from(args)?;
        let explicit_args = capture_sources(&matches)?;
        let command_name = matches.subcommand_name().map(str::to_owned);
        let mut cli = Cli::from_arg_matches_mut(&mut matches)?;
        cli.explicit_args = explicit_args;
        cli.command_name = command_name;
        Ok(cli)
    })
}

/// `--foreign-bbmd`, `--ttl` and `--renewal` reach the transport's
/// foreign-device configuration (#938); `--ttl` also serves `register`,
/// given before or after it.
#[test]
fn the_foreign_device_flags_reach_the_transport() {
    let cli = parse([
        "bacnet",
        "--foreign-bbmd",
        "192.0.2.10",
        "--renewal",
        "20",
        "discover",
        "--ttl",
        "60",
    ])
    .unwrap();
    let args =
        crate::transport::TransportArgs::from_cli(&cli, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)
            .unwrap();
    let foreign = args.foreign_device.unwrap();
    assert_eq!(
        (foreign.bbmd_ip, foreign.bbmd_port, foreign.ttl),
        (Ipv4Addr::new(192, 0, 2, 10), 0xBAC0, 60)
    );
    assert_eq!(
        foreign.renewal_interval,
        Some(std::time::Duration::from_secs(20))
    );

    let cli = parse(["bacnet", "register", "192.0.2.10"]).unwrap();
    assert_eq!((cli.ttl, cli.foreign_bbmd), (300, None));
    let args =
        crate::transport::TransportArgs::from_cli(&cli, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)
            .unwrap();
    assert!(args.foreign_device.is_none());
    let cli = parse(["bacnet", "register", "192.0.2.10", "--ttl", "45"]).unwrap();
    assert_eq!(cli.ttl, 45);
}

/// The SC reconnect flags (#1831): `--sc-failover-url` brings the
/// default reconnect policy, as `--sc-reconnect` does alone; neither
/// leaves reconnect off; the connect timeout is checked at parse time.
#[test]
fn the_sc_reconnect_flags_reach_the_transport() {
    let args = |cli: &Cli| {
        crate::transport::TransportArgs::from_cli(cli, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)
    };
    let cli = parse([
        "bacnet",
        "--sc",
        "--sc-url",
        "wss://hub.example",
        "--sc-failover-url",
        "wss://hub2.example",
        "--sc-connect-timeout",
        "2500",
        "discover",
    ])
    .unwrap();
    let sc = args(&cli).unwrap();
    assert_eq!(sc.sc_failover_url.as_deref(), Some("wss://hub2.example"));
    assert_eq!(
        sc.sc_reconnect.map(|p| p.max_retries),
        Some(bacnet_transport::sc::ScReconnectConfig::default().max_retries)
    );
    assert_eq!(sc.sc_connect_timeout_ms, Some(2500));
    let cli = parse(["bacnet", "--sc", "--sc-reconnect", "discover"]).unwrap();
    assert!(args(&cli).unwrap().sc_reconnect.is_some());
    let cli = parse(["bacnet", "--sc", "discover"]).unwrap();
    let plain = args(&cli).unwrap();
    assert!(plain.sc_reconnect.is_none() && plain.sc_connect_timeout_ms.is_none());
    let same = parse([
        "bacnet",
        "--sc-url",
        "wss://hub.example",
        "--sc-failover-url",
        "WSS://Hub.Example/",
        "discover",
    ])
    .unwrap();
    assert!(args(&same).err().unwrap().contains("differ"), "normalized");
    for bad in ["0", "600001", "soon"] {
        let error = parse(["bacnet", "--sc-connect-timeout", bad, "discover"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation, "{bad}");
    }
}

/// `find` takes the name as a word or with `--name`, never both and
/// never neither, each a usage error (#1872).
#[test]
fn find_takes_its_name_as_a_word_or_a_flag() {
    let name = |cli: Cli| match cli.command {
        Some(Command::Find {
            name, name_flag, ..
        }) => name.or(name_flag),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        name(parse(["bacnet", "find", "AHU-1"]).unwrap()).as_deref(),
        Some("AHU-1")
    );
    assert_eq!(
        name(parse(["bacnet", "find", "--name", "AHU-1"]).unwrap()).as_deref(),
        Some("AHU-1")
    );
    assert_eq!(
        name(parse(["bacnet", "whohas", "Zone Temp", "--wait", "5"]).unwrap()).as_deref(),
        Some("Zone Temp")
    );
    let both = parse(["bacnet", "find", "AHU-1", "--name", "AHU-2"]).unwrap_err();
    assert_eq!(both.kind(), ErrorKind::ArgumentConflict);
    assert_eq!(both.exit_code(), 2);
    let neither = parse(["bacnet", "find"]).unwrap_err();
    assert_eq!(neither.kind(), ErrorKind::MissingRequiredArgument);
    assert_eq!(neither.exit_code(), 2);
}

/// The guard classifies by canonical name, so an alias resolves to it;
/// the guard flags go on either side of the subcommand (#972).
#[test]
fn aliases_give_the_canonical_name_and_guard_flags_are_global() {
    let cli = parse(["bacnet", "wp", "10.0.0.1", "av:1", "pv", "1"]).unwrap();
    assert_eq!(cli.command_name.as_deref(), Some("write"));
    assert_eq!(cli.policy(), crate::core::guard::Policy::default());
    let cli = parse([
        "bacnet",
        "--allow-write",
        "dcc",
        "10.0.0.1",
        "enable",
        "--yes",
    ])
    .unwrap();
    assert_eq!(cli.command_name.as_deref(), Some("control"));
    assert!(cli.allow_write && cli.yes);
    assert_eq!(parse(["bacnet"]).unwrap().command_name, None);
}

#[test]
fn tui_takes_fps_and_log_file_and_global_flags() {
    let cli = parse([
        "bacnet",
        "tui",
        "--fps",
        "5",
        "--log-file",
        "tui.log",
        "-i",
        "10.0.0.5",
    ])
    .unwrap();
    assert_eq!(cli.interface, Some(Ipv4Addr::new(10, 0, 0, 5)));
    let Some(Command::Tui { fps, log_file, .. }) = cli.command else {
        panic!("expected Tui");
    };
    assert_eq!(fps, 5);
    assert_eq!(log_file, Some(PathBuf::from("tui.log")));

    let Some(Command::Tui { fps, log_file, .. }) = parse(["bacnet", "tui"]).unwrap().command else {
        panic!("expected Tui");
    };
    assert_eq!((fps, log_file), (20, None));
    for bad in ["0", "61"] {
        let error = parse(["bacnet", "tui", "--fps", bad]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }
}

#[test]
fn ack_alarm_and_alias_require_both_timestamp_flags() {
    for command in ["ack-alarm", "ack"] {
        let missing_both =
            parse(["bacnet", command, "127.0.0.1", "ai:1", "--state", "1"]).unwrap_err();
        assert_eq!(missing_both.kind(), ErrorKind::MissingRequiredArgument);

        let missing_ack_time = parse([
            "bacnet",
            command,
            "127.0.0.1",
            "ai:1",
            "--state",
            "1",
            "--timestamp",
            "sequence:9",
        ])
        .unwrap_err();
        assert_eq!(missing_ack_time.kind(), ErrorKind::MissingRequiredArgument);
    }
}

#[test]
fn ack_alarm_clap_uses_lossless_shared_timestamp_parser() {
    let cli = parse([
        "bacnet",
        "ack",
        "127.0.0.1",
        "ai:1",
        "--state",
        "3",
        "--source",
        "operator",
        "--timestamp",
        "time:1,2,3,4",
        "--ack-time",
        "datetime:2026,9,2,3;5,6,7,8",
    ])
    .unwrap();
    let Some(Command::AckAlarm {
        state,
        source,
        timestamp,
        ack_time,
        ..
    }) = cli.command
    else {
        panic!("expected AckAlarm");
    };
    assert_eq!(state, 3);
    assert_eq!(source, "operator");
    assert_eq!(
        timestamp,
        BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        })
    );
    assert_eq!(
        ack_time,
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 9,
                day: 2,
                day_of_week: 3,
            },
            time: Time {
                hour: 5,
                minute: 6,
                second: 7,
                hundredths: 8,
            },
        }
    );
}

#[test]
fn ack_alarm_clap_reports_shared_parser_error_before_execution() {
    let error = parse([
        "bacnet",
        "ack-alarm",
        "127.0.0.1",
        "ai:1",
        "--state",
        "1",
        "--timestamp",
        "time:24,0,0,0",
        "--ack-time",
        "sequence:1",
    ])
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
    assert!(
        error
            .to_string()
            .contains("BACnetTimeStamp hour must be 0..=23 or 255 (unspecified), got 24")
    );
}

/// `control`/`dcc` and `reinit` refuse a `--password` over 20 characters
/// at parse time, before anything is resolved or sent, without echoing
/// it (#2085). Twenty multi-octet characters pass.
#[test]
fn device_passwords_over_the_limit_are_refused_at_parse_time() {
    const TWENTY: &str = "éééééééééééééééééééé";
    const TWENTY_ONE: &str = "ééééééééééééééééééééé";
    assert_eq!(TWENTY.chars().count(), 20);
    assert_eq!(TWENTY_ONE.chars().count(), 21);
    for head in [
        ["bacnet", "control", "10.0.0.1", "enable"],
        ["bacnet", "dcc", "10.0.0.1", "enable"],
        ["bacnet", "reinit", "10.0.0.1", "warmstart"],
    ] {
        let [a, b, c, d] = head;
        let cli = parse([a, b, c, d, "--password", TWENTY]).unwrap();
        let Some(Command::Control { password, .. } | Command::Reinit { password, .. }) =
            cli.command
        else {
            panic!("unexpected command for {head:?}");
        };
        assert_eq!(password.as_deref(), Some(TWENTY));

        let error = parse([a, b, c, d, "--password", TWENTY_ONE]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation, "{head:?}");
        let text = error.to_string();
        assert!(text.contains("20-character limit"), "{text}");
        assert!(!text.contains("éé"), "the password leaked: {text}");

        // An empty one is refused too: leave it out instead (#2094).
        let error = parse([a, b, c, d, "--password", ""]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation, "{head:?}");
        assert!(error.to_string().contains("is empty"), "{error}");
    }
}
