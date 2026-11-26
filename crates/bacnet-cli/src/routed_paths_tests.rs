use super::*;

// Parse where `main` does, on a thread with a larger stack (#953).
fn parse<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
    crate::parse_on_large_stack(move || {
        use clap::{CommandFactory, FromArgMatches};
        let mut matches = Cli::command().try_get_matches_from(args)?;
        crate::args::capture_sources(&matches)?;
        Cli::from_arg_matches_mut(&mut matches)
    })
}

#[test]
fn a_reserved_router_network_is_rejected_by_clap_before_ipv6_resolution() {
    for network in ["0", "65535"] {
        let error = parse([
            "bacnet",
            "whois-router",
            "--dnet",
            network,
            "--router",
            "[::1]:47808",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }
}

fn limit(router: &str, network: u16, max_npdu: u16) -> LimitArg {
    LimitArg {
        router: router.to_owned(),
        network,
        max_npdu,
    }
}

fn row(source: &'static str, learned: Option<u16>, seen: bool) -> RoutedPathStatus {
    use bacnet_ops::RoutedPathLimitSource::{Configured, Default, Learned};
    RoutedPathStatus {
        path: bacnet_ops::RoutedPath::new(AddressStyle::Bip, &[192, 168, 1, 20, 0xBA, 0xC0], 5)
            .unwrap(),
        max_npdu: 128,
        source: match source {
            "configured" => Configured,
            "learned" => Learned,
            _ => Default,
        },
        configured_max_npdu: Some(128),
        learned_max_npdu: learned,
        below_minimum_reject_seen: seen,
    }
}

/// The router is everything before the last `/`, so every address form
/// fits; the network is 1 to 65534 and the limit a u16.
#[test]
fn the_flag_takes_every_router_form() {
    for (text, expected) in [
        (
            "192.168.1.20:47808/5=480",
            limit("192.168.1.20:47808", 5, 480),
        ),
        ("192.168.1.20/5=480", limit("192.168.1.20", 5, 480)),
        ("mstp:3/7=128", limit("mstp:3", 7, 128)),
        (
            "bip6:0x0004D2/65534=1497",
            limit("bip6:0x0004D2", 65534, 1497),
        ),
        (
            "[2001:db8::10]:47809/1=60",
            limit("[2001:db8::10]:47809", 1, 60),
        ),
        ("02:00:00:00:00:01/9=0", limit("02:00:00:00:00:01", 9, 0)),
    ] {
        assert_eq!(parse_limit_arg(text), Ok(expected), "{text}");
    }
    for (text, why) in [
        ("192.168.1.20:47808", "is not ROUTER/NETWORK=MAX_NPDU"),
        ("192.168.1.20:47808=480", "is not ROUTER/NETWORK=MAX_NPDU"),
        ("/5=480", "names no router"),
        ("192.168.1.20/0=480", "'0' is not a remote network number"),
        (
            "192.168.1.20/65535=480",
            "'65535' is not a remote network number",
        ),
        ("192.168.1.20/x=480", "'x' is not a remote network number"),
        (
            "192.168.1.20/5=65536",
            "'65536' is not a number of NPDU octets",
        ),
        ("192.168.1.20/5=", "'' is not a number of NPDU octets"),
    ] {
        let error = parse_limit_arg(text).unwrap_err();
        assert!(error.contains(why), "{text}: {error}");
    }
}

/// The flag repeats before the command, and is refused after it rather
/// than half kept; plain `bacnet` and the commands with no client to set it
/// on refuse it rather than ignore it.
#[test]
fn the_flag_repeats_on_one_shots_and_is_refused_where_nothing_uses_it() {
    let cli = parse([
        "bacnet",
        "--routed-path-limit",
        "10.0.0.1/5=480",
        "--routed-path-limit",
        "mstp:3/7=128",
        "read",
        "5:1234",
        "ai:1",
        "pv",
    ])
    .unwrap();
    assert_eq!(
        cli.routed_path_limit,
        [limit("10.0.0.1", 5, 480), limit("mstp:3", 7, 128)]
    );
    let after = parse([
        "bacnet",
        "read",
        "5:1234",
        "ai:1",
        "pv",
        "--routed-path-limit",
        "mstp:3/7=128",
    ])
    .unwrap_err();
    assert_eq!(after.kind(), clap::error::ErrorKind::UnknownArgument);
    assert!(
        parse(["bacnet", "read", "5:1234", "ai:1", "pv"])
            .unwrap()
            .routed_path_limit
            .is_empty()
    );
    for args in [
        ["bacnet", "--routed-path-limit", "10.0.0.1/5=480", "tui"],
        [
            "bacnet",
            "--routed-path-limit",
            "10.0.0.1/5=480",
            "mstp-ports",
        ],
        [
            "bacnet",
            "--routed-path-limit",
            "10.0.0.1/5=480",
            "sc-check",
        ],
        [
            "bacnet",
            "--routed-path-limit",
            "10.0.0.1/5=480",
            "--no-tui",
        ],
    ] {
        let error = parse(args).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "{args:?}"
        );
        assert!(
            error
                .to_string()
                .contains("--routed-path-limit applies to the one-shot commands"),
            "{args:?}: {error}"
        );
    }
    let error = parse([
        "bacnet",
        "--routed-path-limit",
        "10.0.0.1/0=480",
        "whois-router",
    ])
    .unwrap_err();
    assert!(
        error.to_string().contains("not a remote network number"),
        "{error}"
    );
}

/// The summary shows what the command line did not ask for at once, and
/// limits set by flag only with `-v`.
#[test]
fn the_summary_shows_for_what_was_not_asked_for() {
    let configured = Summary::new(vec![row("configured", None, false)], 0, 0, 0, false);
    assert!(!configured.shown(0));
    assert!(configured.shown(1));
    assert!(!Summary::new(Vec::new(), 0, 0, 0, false).shown(2));
    assert!(Summary::new(vec![row("learned", Some(300), false)], 0, 0, 0, false).shown(0));
    assert!(Summary::new(Vec::new(), 1, 0, 0, false).shown(0));
    assert!(Summary::new(vec![row("default", None, true)], 0, 1, 0, false).shown(0));
    // An answer refused for the path it came by is shown unasked (#2831).
    assert!(Summary::new(Vec::new(), 0, 0, 1, false).shown(0));
}

/// The hint comes with a timeout after a dropped below-minimum reject, and
/// names the flag for each path that drew one.
#[test]
fn a_timeout_after_a_below_minimum_drop_names_the_flag() {
    let paths = vec![row("default", None, true)];
    assert_eq!(Summary::new(paths.clone(), 0, 1, 0, false).hint, None);
    assert_eq!(Summary::new(paths.clone(), 3, 0, 0, true).hint, None);
    let hint = Summary::new(paths, 0, 1, 0, true).hint.unwrap();
    assert!(
        hint.contains("--routed-path-limit 192.168.1.20:47808/5=SIZE"),
        "{hint}"
    );
}

/// JSON is one line with every key, hint included; the table has a row a
/// path, the counts and the hint.
#[test]
fn the_summary_renders_as_one_json_line_or_a_table() {
    let summary = Summary::new(vec![row("default", None, true)], 2, 1, 3, true);
    let json = summary.render(OutputFormat::Json);
    assert!(!json.contains('\n'));
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["routed_paths"],
        serde_json::json!([{
            "router": "192.168.1.20:47808",
            "network": 5,
            "max_npdu": 128,
            "source": "default",
            "configured_max_npdu": 128,
            "learned_max_npdu": null,
            "below_minimum_reject_seen": true,
        }])
    );
    assert_eq!(
        (
            &value["too_long_reject_drops"],
            &value["below_minimum_reject_drops"]
        ),
        (&serde_json::json!(2), &serde_json::json!(1))
    );
    assert_eq!(value["off_path_answer_drops"], serde_json::json!(3));
    assert!(
        value["hint"]
            .as_str()
            .unwrap()
            .contains("--routed-path-limit")
    );
    let quiet = Summary::new(Vec::new(), 1, 0, 0, false).render(OutputFormat::Json);
    let value: serde_json::Value = serde_json::from_str(&quiet).unwrap();
    assert_eq!(value["hint"], serde_json::Value::Null);

    let table = summary.render(OutputFormat::Table);
    for text in [
        "Routed paths:",
        "192.168.1.20:47808",
        "Small request refused",
        "Too-long rejects not believed: 2 that could answer",
        "1 for a request of 228 NPDU octets or fewer",
        "Answers refused for the path they came by: 3 not from the request's router or device",
        "Hint: a router refused",
    ] {
        assert!(table.contains(text), "{text}: {table}");
    }
    let empty = Summary::new(Vec::new(), 1, 0, 0, false).render(OutputFormat::Table);
    assert!(
        empty.contains("none beyond the default of 228 NPDU octets"),
        "{empty}"
    );
}

/// A timeout is found anywhere in the error's chain; another error is not
/// one.
#[test]
fn only_a_timeout_is_a_timeout() {
    let timeout: CliResult = Err(Box::new(bacnet_types::error::Error::Timeout(
        std::time::Duration::from_secs(6),
    )));
    assert!(timed_out(&timeout));
    let tsm: CliResult = Err(Box::new(bacnet_types::error::Error::abort(
        bacnet_types::enums::AbortReason::TSM_TIMEOUT,
    )));
    assert!(timed_out(&tsm));
    let abort: CliResult = Err(Box::new(bacnet_types::error::Error::abort(
        bacnet_types::enums::AbortReason::SEGMENTATION_NOT_SUPPORTED,
    )));
    assert!(!timed_out(&abort));
    // An OpsError's source skips the client error it wraps.
    let ops: CliResult = Err(Box::new(bacnet_ops::OpsError::from(
        bacnet_types::error::Error::abort(bacnet_types::enums::AbortReason::TSM_TIMEOUT),
    )));
    assert!(timed_out(&ops));
    let refused: CliResult = Err(Box::new(bacnet_ops::OpsError::from(
        bacnet_types::error::Error::abort(
            bacnet_types::enums::AbortReason::SEGMENTATION_NOT_SUPPORTED,
        ),
    )));
    assert!(!timed_out(&refused));
    let other: CliResult = Err("device 5:1234 did not answer".into());
    assert!(!timed_out(&other));
    assert!(!timed_out(&Ok(())));
}
