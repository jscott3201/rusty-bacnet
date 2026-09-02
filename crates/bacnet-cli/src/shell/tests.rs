use super::*;

#[test]
fn tokenize_simple() {
    let tokens = tokenize("read 192.168.1.10 ai:1 pv");
    assert_eq!(tokens, vec!["read", "192.168.1.10", "ai:1", "pv"]);
}

#[test]
fn tokenize_quoted_string() {
    let tokens = tokenize("write 10.0.1.5 av:1 pv \"hello world\"");
    assert_eq!(
        tokens,
        vec!["write", "10.0.1.5", "av:1", "pv", "\"hello world\""]
    );
}

#[test]
fn tokenize_empty() {
    let tokens = tokenize("");
    assert!(tokens.is_empty());
}

#[test]
fn tokenize_extra_whitespace() {
    let tokens = tokenize("  read   10.0.1.5   ai:1   pv  ");
    assert_eq!(tokens, vec!["read", "10.0.1.5", "ai:1", "pv"]);
}

#[test]
fn shell_helper_completions() {
    let helper = ShellHelper::new();
    assert!(!helper.commands.is_empty());
    assert!(!helper.object_types.is_empty());
    assert!(!helper.properties.is_empty());
}

fn ack_args(options: &[&str]) -> Vec<String> {
    ["127.0.0.1", "ai:1"]
        .into_iter()
        .chain(options.iter().copied())
        .map(str::to_string)
        .collect()
}

#[test]
fn shell_ack_alarm_requires_both_timestamps_before_dispatch() {
    let missing_timestamp = ack_args(&["--state", "1", "--ack-time", "sequence:2"]);
    assert_eq!(
        admin::parse_ack_alarm_arguments(&missing_timestamp).unwrap_err(),
        "--timestamp is required"
    );

    let missing_ack_time = ack_args(&["--state", "1", "--timestamp", "sequence:2"]);
    assert_eq!(
        admin::parse_ack_alarm_arguments(&missing_ack_time).unwrap_err(),
        "--ack-time is required"
    );
}

#[test]
fn shell_ack_alarm_uses_shared_parser_and_preserves_options() {
    let args = ack_args(&[
        "--state",
        "3",
        "--source",
        "operator",
        "--timestamp",
        "time:1,2,3,4",
        "--ack-time",
        "datetime:2026,9,2,3;5,6,7,8",
    ]);
    let parsed = admin::parse_ack_alarm_arguments(&args).unwrap();
    assert_eq!(parsed.state, 3);
    assert_eq!(parsed.source, "operator");
    assert_eq!(
        parsed.timestamp,
        bacnet_types::primitives::BACnetTimeStamp::Time(bacnet_types::primitives::Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        })
    );

    let invalid = ack_args(&[
        "--state",
        "1",
        "--timestamp",
        "time:24,0,0,0",
        "--ack-time",
        "sequence:2",
    ]);
    assert_eq!(
        admin::parse_ack_alarm_arguments(&invalid).unwrap_err(),
        timestamp::parse_bacnet_timestamp("time:24,0,0,0").unwrap_err()
    );
}
