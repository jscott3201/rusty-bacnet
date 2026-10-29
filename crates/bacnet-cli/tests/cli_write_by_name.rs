//! Real CLI processes writing values by name (#2075): what `read` prints
//! for an enumeration, a named bit string or a date-time is what `write`
//! takes, so a value read pastes back as a write. Constructed values
//! (schedule days, calendar entries, destinations) are typed as JSON.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::primitives::PropertyValue;
use serde_json::Value;
use support::lab::{lab_database, lab_identity};

/// The lab device, with a Schedule, a Calendar and a Notification Class.
fn database() -> bacnet_objects::database::ObjectDatabase {
    let mut db = lab_database(1234);
    let schedule = ScheduleObject::new(1, "Occupancy", PropertyValue::Real(60.0)).unwrap();
    db.add(Box::new(schedule)).unwrap();
    db.add(Box::new(CalendarObject::new(1, "Holidays").unwrap()))
        .unwrap();
    db.add(Box::new(NotificationClass::new(2, "Desk").unwrap()))
        .unwrap();
    db
}

async fn server() -> (BACnetServer<BipTransport>, String) {
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(database())
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (server, target)
}

async fn cli(target: &str, args: &[&str]) -> std::process::Output {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        "json",
        "--allow-write",
    ];
    all.push(args[0]);
    all.push(target);
    all.extend(&args[1..]);
    support::run(all).await
}

async fn ok(target: &str, args: &[&str]) -> String {
    let output = cli(target, args).await;
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The text `read` prints for one property.
async fn read(target: &str, object: &str, property: &str) -> String {
    let output = ok(target, &["read", object, property]).await;
    let json: Value = serde_json::from_str(&output).unwrap();
    json["value"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn writes_take_names_bits_and_date_times_as_read_prints_them() {
    let (mut server, target) = server().await;
    for (object, property, typed, printed) in [
        // A plain number for an analog value is sent as a REAL, which
        // the device takes; an Unsigned it refuses.
        ("av:1", "present-value", "72", "72"),
        ("av:1", "cov-increment", "2", "2"),
        // An enumeration by name, in any spelling. `read` adds the
        // object's state text, and that form writes back too (#2132).
        ("bv:1", "present-value", "ACTIVE", "active (Active)"),
        ("bv:1", "present-value", "inactive", "inactive (Inactive)"),
        // A named bit string by its bits' names.
        (
            "ai:1",
            "event-enable",
            "to-offnormal | to-normal",
            "to-offnormal | to-normal",
        ),
        ("ai:1", "event-enable", "to-fault", "to-fault"),
        // A date-time as read prints it.
        (
            "trend-log:1",
            "start-time",
            "2026-10-25 Sun 14:30:05",
            "2026-10-25 Sun 14:30:05",
        ),
        (
            "trend-log:1",
            "start-time",
            "2026-12-31 23:59",
            "2026-12-31 23:59",
        ),
    ] {
        ok(&target, &["write", object, property, typed]).await;
        assert_eq!(read(&target, object, property).await, printed, "{typed}");
        // What read printed writes back unchanged.
        ok(&target, &["write", object, property, printed]).await;
        assert_eq!(read(&target, object, property).await, printed, "{printed}");
    }
    // writem takes names too.
    ok(
        &target,
        &[
            "writem",
            "bv:1",
            "pv=active",
            "ai:1",
            "event-enable=to-fault|to-normal",
        ],
    )
    .await;
    assert_eq!(read(&target, "bv:1", "pv").await, "active (Active)");
    assert_eq!(
        read(&target, "ai:1", "event-enable").await,
        "to-fault | to-normal"
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_mistyped_name_is_refused_with_the_choices() {
    let (mut server, target) = server().await;
    let output = cli(&target, &["write", "bv:1", "pv", "actve"]).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown BinaryPV 'actve'; expected one of: inactive, active"),
        "{stderr}"
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn constructed_values_take_json() {
    let (mut server, target) = server().await;
    for (object, property, json, printed) in [
        (
            "schedule:1",
            "weekly-schedule[2]",
            r#"[["07:00", 72.5], {"time": "18:00", "value": null}]"#,
            "Tue 07:00 = 72.5, 18:00 = null",
        ),
        (
            "calendar:1",
            "date-list",
            r#"[{"kind": "date", "date": "2026-12-25"}]"#,
            "[2026-12-25]",
        ),
        (
            "notification-class:2",
            "recipient-list",
            r#"[{"recipient": "device:99", "process_identifier": 7, "valid_days": ["mon", "tue", "wed", "thu", "fri"], "from_time": "08:00", "to_time": "17:00", "issue_confirmed_notifications": true}]"#,
            "[device 99 process 7: Mon-Fri 08:00-17:00, confirmed, to-offnormal | to-fault | to-normal]",
        ),
    ] {
        ok(&target, &["write", object, property, json]).await;
        assert_eq!(read(&target, object, property).await, printed, "{json}");
    }
    let output = cli(
        &target,
        &[
            "write",
            "calendar:1",
            "date-list",
            r#"[{"kind": "date", "day": "2026-12-25"}]"#,
        ],
    )
    .await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown field 'day'"), "{stderr}");
    server.stop().await.unwrap();
}
