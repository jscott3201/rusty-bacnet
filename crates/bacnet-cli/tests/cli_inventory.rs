//! `bacnet inventory` against the lab device (#2560): the table, the JSON
//! and the CSV it prints, what it costs on the wire, and its exit status.
//! A relay between the CLI and the device counts the confirmed requests.
#[allow(dead_code)]
mod support;

use std::process::Output;

use serde_json::Value;
use support::lab::{large_database, named_database};
use support::relay::{relay, server, serving};

async fn inventory(args: &[&str]) -> Output {
    inventory_within("2000", args).await
}

/// [`inventory`] with an APDU timeout of `timeout` ms.
async fn inventory_within(timeout: &str, args: &[&str]) -> Output {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--timeout",
        timeout,
        "inventory",
    ];
    all.extend(args);
    support::run(all).await
}

/// The output's stdout, its line ends as Unix writes them (Windows CI
/// sees `\r\n`).
fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .replace("\r\n", "\n")
}

/// JSON, by address: a bare document on stdout with the device, every
/// object of every type and typed fields; the note goes to stderr. One
/// request to learn the instance, then the summary, the Object_List and
/// two batches for the eleven objects.
#[tokio::test]
async fn the_json_is_the_whole_inventory() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let output = inventory(&[&target, "--json"]).await;
    let json: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["address"], target.as_str());
    assert_eq!(json["device"]["instance"], 1234);
    assert_eq!(json["device"]["name"], "Lab device 1234");
    assert_eq!(
        json["device"]["properties"]["model-name"]["decoded"],
        "Bench-1"
    );
    assert!(json["stopped"].is_null());
    assert_eq!(json["unread"], serde_json::json!([]));
    let points = json["points"].as_array().unwrap();
    let objects: Vec<String> = points
        .iter()
        .map(|point| {
            format!(
                "{}:{}",
                point["object"]["type"].as_str().unwrap(),
                point["object"]["instance"]
            )
        })
        .collect();
    let expected = [
        "device:1234",
        "analog-input:1",
        "analog-input:2",
        "analog-output:1",
        "analog-value:1",
        "binary-input:1",
        "binary-output:1",
        "binary-value:1",
        "multi-state-value:1",
        "notification-class:1",
        "trend-log:1",
    ];
    assert_eq!(objects, expected, "every type, in Object_List order");
    let zone = &points[1];
    assert_eq!(zone["name"], "Zone Temp");
    assert_eq!(zone["device"], target.as_str());
    assert_eq!(zone["device_instance"], 1234);
    assert_eq!(zone["units"]["name"], "degrees-fahrenheit");
    assert_eq!(zone["commandable"], false);
    assert!(zone["present_value"].is_null(), "no value unless asked");
    assert_eq!(points[3]["commandable"], true, "an output");
    assert_eq!(points[8]["number_of_states"], 3);
    assert_eq!(points[10]["name"], "Zone Temp Log");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let note = format!("Read 11 objects of device 1234 (Lab device 1234) at {target}; 0 with");
    assert!(stderr.contains(&note), "{stderr}");
    assert_eq!(
        seen.lock().unwrap().len(),
        1 + 2 + 2,
        "wildcard read, summary, list, two batches"
    );
    relay.abort();
    server.stop().await.unwrap();
}

/// `--values` adds each object's Present_Value and Status_Flags in the
/// same requests, the text naming units; `--types` narrows the objects
/// read; `--settings` adds the relinquish default.
#[tokio::test]
async fn values_types_and_settings() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let args = [target.as_str(), "--types", "ai,ao", "--values", "--json"];
    let json: Value = serde_json::from_str(&stdout(&inventory(&args).await)).unwrap();
    let points = json["points"].as_array().unwrap();
    assert_eq!(points.len(), 3, "two inputs and the output");
    assert_eq!(points[0]["present_value"]["value"], "72.5 °F");
    assert_eq!(points[0]["present_value"]["decoded"], 72.5);
    let flags = &points[0]["status_flags"]["decoded"]["names"];
    assert_eq!(
        flags,
        &serde_json::json!(["in-alarm", "fault", "out-of-service"])
    );
    assert_eq!(points[2]["present_value"]["decoded"], 55.0);
    assert!(points[2]["relinquish_default"].is_null());
    assert_eq!(seen.lock().unwrap().len(), 1 + 2 + 1, "one batch of three");

    let args = [target.as_str(), "--types", "ao", "--settings", "--json"];
    let json: Value = serde_json::from_str(&stdout(&inventory(&args).await)).unwrap();
    assert_eq!(json["points"][0]["relinquish_default"]["decoded"], 0.0);
    assert!(json["points"][0]["present_value"].is_null());
    relay.abort();
    server.stop().await.unwrap();
}

/// `--format csv`: the documented header, then one row an object.
#[tokio::test]
async fn the_csv_is_one_row_an_object() {
    let (mut server, device) = server().await;
    let (target, _, relay) = relay(device, usize::MAX).await;
    let args = [target.as_str(), "--values", "--format", "csv"];
    let csv = stdout(&inventory(&args).await);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "device,device_instance,object,name,description,units,state_texts,number_of_states,\
         commandable,relinquish_default,cov_increment,min_present_value,max_present_value,\
         present_value,status_flags,problems"
    );
    assert_eq!(lines.len(), 1 + 11);
    let zone = format!(
        "{target},1234,analog-input:1,Zone Temp,,degrees-fahrenheit,,,false,,,,,72.5 °F,\
         in-alarm | fault | out-of-service,"
    );
    assert_eq!(lines[2], zone);
    assert!(
        lines[9].contains(",multi-state-value:1,Mode,,,State 1|State 2|State 3,3,true,"),
        "{}",
        lines[9]
    );
    // --json outranks --format csv, as it does --format table.
    let both = [target.as_str(), "--format", "csv", "--json"];
    assert!(stdout(&inventory(&both).await).starts_with('{'));
    relay.abort();
    server.stop().await.unwrap();
}

/// `--format table`: a row an object for people, the note on stderr.
#[tokio::test]
async fn the_table_lists_the_objects() {
    let (mut server, device) = server().await;
    let (target, _, relay) = relay(device, usize::MAX).await;
    let args = [target.as_str(), "--values", "--format", "table"];
    let output = inventory(&args).await;
    let text = stdout(&output);
    for column in ["Object", "Name", "Units", "Value", "Status", "Commandable"] {
        assert!(text.contains(column), "{column}\n{text}");
    }
    let row = |object: &str| {
        text.lines()
            .find(|line| line.contains(object))
            .unwrap_or_else(|| panic!("{object}\n{text}"))
    };
    let zone = row("analog-input:1");
    for cell in ["Zone Temp", "degrees-fahrenheit", "72.5 °F", "no"] {
        assert!(zone.contains(cell), "{cell}\n{zone}");
    }
    assert!(row("analog-output:1").contains("yes"));
    assert!(row("binary-value:1").contains("Inactive / Active"));
    assert!(!text.contains("Problems"), "none had one\n{text}");
    assert!(!text.contains("Read 11 objects"), "the note is on stderr");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Read 11 objects of device 1234"),
        "{stderr}"
    );
    relay.abort();
    server.stop().await.unwrap();
}

/// A device that stops answering partway: what was read is still printed,
/// with the objects not read and why, and the exit status is 1. One that
/// never answers prints nothing and exits 1.
#[tokio::test]
async fn an_incomplete_inventory_exits_1() {
    // The Device object and forty analog values.
    let (mut server, device) = serving(large_database(1234, 40)).await;
    // The wildcard read, the summary, the list and the first call's two
    // batches (the Device and seven values, then eight values).
    let (target, seen, front) = relay(device, 5).await;
    let output = inventory_within("300", &[&target, "--json"]).await;
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["points"].as_array().unwrap().len(), 16);
    assert_eq!(json["unread"].as_array().unwrap().len(), 25);
    assert_eq!(json["unread"][0]["instance"], 16);
    assert!(json["stopped"].is_string(), "{text}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the inventory of device 1234 stopped early ("),
        "{stderr}"
    );
    assert!(stderr.contains("25 objects were not read"), "{stderr}");
    assert_eq!(seen.lock().unwrap().len(), 6, "nothing after the timeout");
    front.abort();

    let (target, _, relay) = relay(device, 0).await;
    let output = inventory_within("300", &[&target, "--json"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "nothing was read");
    relay.abort();
    server.stop().await.unwrap();
}

/// A type the device has none of is an empty inventory, exit 0, and
/// stderr says so; a type name that isn't one is refused before anything
/// is sent, and a missing target is a usage error.
#[tokio::test]
async fn no_object_and_bad_options() {
    let (mut server, device) = server().await;
    let (target, seen, relay) = relay(device, usize::MAX).await;
    let output = inventory(&[&target, "--types", "mso", "--format", "csv"]).await;
    let csv = stdout(&output);
    assert_eq!(csv.lines().count(), 1, "the header alone: {csv}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Device 1234 has no object of the selected types."),
        "{stderr}"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);

    support::failure(
        &inventory(&[&target, "--types", "nope"]).await,
        "unknown object type",
    );
    assert_eq!(seen.lock().unwrap().len(), 3, "refused before sending");
    let usage = inventory(&[]).await;
    assert_eq!(usage.status.code(), Some(2));
    relay.abort();
    server.stop().await.unwrap();
}

/// CSV guards device text a spreadsheet would run as a formula with an
/// apostrophe; `--csv-exact` writes it as the device sent it, and JSON is
/// always exact.
#[tokio::test]
async fn the_csv_guards_formulas_unless_exact() {
    let names = ["=1+1", "-Offset", "Plain"];
    let (mut server, device) = serving(named_database(1234, &names)).await;
    let (target, _, relay) = relay(device, usize::MAX).await;
    let name = |csv: &str, row: usize| -> String {
        let line = csv.lines().nth(row).unwrap_or_else(|| panic!("{csv}"));
        line.split(',').nth(3).unwrap().to_string()
    };
    let guarded = stdout(&inventory(&[&target, "--types", "av", "--format", "csv"]).await);
    assert_eq!(name(&guarded, 1), "'=1+1", "{guarded}");
    assert_eq!(name(&guarded, 2), "'-Offset");
    assert_eq!(name(&guarded, 3), "Plain");
    let args = [
        target.as_str(),
        "--types",
        "av",
        "--format",
        "csv",
        "--csv-exact",
    ];
    let exact = stdout(&inventory(&args).await);
    assert_eq!(name(&exact, 1), "=1+1", "{exact}");
    assert_eq!(name(&exact, 2), "-Offset");
    let json = stdout(&inventory(&[&target, "--types", "av", "--json"]).await);
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["points"][0]["name"], "=1+1");
    relay.abort();
    server.stop().await.unwrap();
}
