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

#[test]
fn tokenized_documented_ack_alarm_preserves_quoted_datetime() {
    let tokens = tokenize(
        "ack-alarm 192.168.1.100 ai:1 --state 1 --timestamp sequence:417 \
         --ack-time \"datetime:2026,9,2,3;14,30,00,00\"",
    );
    assert_eq!(tokens[0], "ack-alarm");
    let parsed = admin::parse_ack_alarm_arguments(&tokens[1..]).unwrap();
    assert_eq!(
        parsed.timestamp,
        bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(417)
    );
    assert_eq!(
        parsed.time_of_acknowledgment,
        bacnet_types::primitives::BACnetTimeStamp::DateTime {
            date: bacnet_types::primitives::Date {
                year: 126,
                month: 9,
                day: 2,
                day_of_week: 3,
            },
            time: bacnet_types::primitives::Time {
                hour: 14,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        }
    );
}

#[test]
fn tokenized_ack_alarm_accepts_one_quoted_event_timestamp_pair() {
    let tokens = tokenize(
        "ack 192.168.1.100 ai:1 --state 3 --timestamp \"time:1,2,3,4\" \
         --ack-time sequence:9",
    );
    assert_eq!(tokens[0], "ack");
    let parsed = admin::parse_ack_alarm_arguments(&tokens[1..]).unwrap();
    assert_eq!(
        parsed.timestamp,
        bacnet_types::primitives::BACnetTimeStamp::Time(bacnet_types::primitives::Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        })
    );
    assert_eq!(
        parsed.time_of_acknowledgment,
        bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(9)
    );
}

/// The shell's `[ip]:port` target is the VMAC of the node there, found with
/// a Virtual-Address-Resolution (Clause U.5), and `status` shows the
/// shell's own VMAC with the address it is bound to.
#[tokio::test]
async fn shell_bip6_address_target_resolves_to_the_vmac_there() {
    let mut server =
        bacnet_transport::bip6::Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 0, Some(1234));
    let _server_rx = server.start().await.unwrap();
    let port = server.local_address().unwrap().port();
    let mut client = BACnetClient::bip6_builder()
        .interface(std::net::Ipv6Addr::LOCALHOST)
        .port(0)
        // An instance VMAC: a random one needs a multicast probe at start,
        // which Linux loopback can't carry.
        .device_instance(4321)
        .build()
        .await
        .unwrap();
    let target = format!("[::1]:{port}");
    assert_eq!(
        resolve_target_mac(&client, &target).await.unwrap(),
        [0x00, 0x04, 0xD2]
    );
    let bound = resolve::as_bip6(client.transport())
        .and_then(|bip6| bip6.local_address())
        .unwrap();
    let (label, line) = local_address_line(client.local_mac(), Some(bound));
    assert_eq!(label, "Local address:");
    // Instance 4321 is VMAC 00 10 E1 (H.7.2).
    assert_eq!(line, format!("bip6:0x0010E1 ({bound})"));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[test]
fn status_shows_each_local_address_as_a_target() {
    let bound = "[::1]:47808".parse().unwrap();
    assert_eq!(
        local_address_line(&[0x00, 0x04, 0xD2], Some(bound)),
        ("Local address:", "bip6:0x0004D2 ([::1]:47808)".to_string())
    );
    assert_eq!(
        local_address_line(&[0x00, 0x04, 0xD2], None),
        ("Local address:", "bip6:0x0004D2".to_string())
    );
    assert_eq!(
        local_address_line(&[10, 0, 0, 1, 0xBA, 0xC0], None),
        ("Local address:", "10.0.0.1:47808".to_string())
    );
    assert_eq!(
        local_address_line(&[7], None),
        ("Local MAC:", "[07]".to_string())
    );
}
