//! Device strings in one-shot output (#2144): a device whose names,
//! description, state texts and values hold a right-to-left override, an
//! escape sequence, a bell, a newline, a line separator and an emoji
//! family. Table output prints them cleaned, as the terminal UI shows
//! them, the joiner kept; JSON keeps the device's exact text.
#[allow(dead_code)]
mod support;

use std::net::{Ipv4Addr, SocketAddr};

use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use serde_json::Value;
use support::lab::{lab_database, lab_identity};

const INSTANCE: u32 = 2144;

/// The family emoji: man, zero width joiner, woman.
const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}";

/// Device text with one of each: an override, an escape sequence that
/// clears the screen, a bell, a newline, a line separator, and the family.
fn hostile(word: &str) -> String {
    format!("{word}\u{202E}A\u{1b}[2JB\u{7}C\nD\u{2028}E {FAMILY}")
}

/// [`hostile`] as table output prints it.
fn cleaned(word: &str) -> String {
    format!("{word}A [2JB C D E {FAMILY}")
}

/// Table output with none of the hostile characters left, the cleaned
/// `words` in it, and the joiner kept.
fn assert_clean(table: &str, words: &[&str]) {
    for bad in ["\u{202E}", "\u{1b}[2J", "\u{7}", "\u{2028}"] {
        assert!(!table.contains(bad), "{bad:?} in {table:?}");
    }
    for word in words {
        assert!(table.contains(&cleaned(word)), "{word}: {table:?}");
    }
    assert!(table.contains(FAMILY), "{table:?}");
}

/// The lab device, a B/IP BBMD on loopback (so `find` reaches it as a
/// foreign device), with Multi-State Value 9 named `hostile("Name")`,
/// described `hostile("Desc")`, at state 2 of State_Text `Off`,
/// `hostile("State")`, `High`; and CharacterString Value 1 holding
/// `hostile("Value")`.
async fn server() -> (BACnetServer<BipTransport>, SocketAddr) {
    let text = |text: String| PropertyValue::CharacterString(text);
    let mut msv = MultiStateValueObject::new(9, hostile("Name"), 3).unwrap();
    msv.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        text(hostile("Desc")),
        None,
    )
    .unwrap();
    msv.write_property(
        PropertyIdentifier::STATE_TEXT,
        None,
        PropertyValue::List(vec![
            text("Off".into()),
            text(hostile("State")),
            text("High".into()),
        ]),
        None,
    )
    .unwrap();
    msv.set_relinquish_default(2).unwrap();
    let mut csv = CharacterStringValueObject::new(1, "Message").unwrap();
    csv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        text(hostile("Value")),
        Some(16),
    )
    .unwrap();
    let mut db = lab_database(INSTANCE);
    db.add(Box::new(msv)).unwrap();
    db.add(Box::new(csv)).unwrap();
    let server = BACnetServer::bip_builder()
        .device(lab_identity(INSTANCE))
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]])));
    (server, address)
}

fn args<'a>(format: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
    ];
    all.extend(rest);
    all
}

/// The CLI's stdout, after it succeeded.
async fn cli(format: &str, rest: &[&str]) -> String {
    let output = support::run(args(format, rest)).await;
    assert!(
        output.status.success(),
        "{rest:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `read` and `readm` print the name, description, state text and string
/// value cleaned in a table, and exactly as sent in JSON, `value` and
/// `decoded` alike.
#[tokio::test]
async fn reads_print_device_text_cleaned_in_tables_and_exact_in_json() {
    let (mut server, device) = server().await;
    let target = device.to_string();
    for (object, property, word) in [
        ("msv:9", "object-name", "Name"),
        ("msv:9", "description", "Desc"),
        ("msv:9", "state-text[2]", "State"),
        ("characterstring-value:1", "present-value", "Value"),
    ] {
        let table = cli("table", &["read", &target, object, property]).await;
        assert_clean(&table, &[word]);
        let json: Value =
            serde_json::from_str(&cli("json", &["read", &target, object, property]).await).unwrap();
        assert_eq!(json["value"], format!("\"{}\"", hostile(word)), "{json}");
        assert_eq!(json["decoded"], hostile(word), "{json}");
    }
    // The state named by its text: cleaned in both forms, as #2132 has it.
    let table = cli("table", &["read", &target, "msv:9", "pv"]).await;
    assert!(
        table.contains(&format!("2 ({})", cleaned("State"))),
        "{table:?}"
    );

    let readm = [
        "readm",
        &target,
        "msv:9",
        "object-name,description,state-text",
    ];
    let table = cli("table", &readm).await;
    assert_clean(&table, &["Name", "Desc", "State"]);
    let json: Value = serde_json::from_str(&cli("json", &readm).await).unwrap();
    let values: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["value"].as_str().unwrap())
        .collect();
    assert_eq!(values[0], format!("\"{}\"", hostile("Name")));
    assert_eq!(values[1], format!("\"{}\"", hostile("Desc")));
    assert_eq!(
        values[2],
        format!("[\"Off\", \"{}\", \"High\"]", hostile("State"))
    );

    let all = cli("table", &["read", &target, "msv:9", "all"]).await;
    assert_clean(&all, &["Name", "Desc", "State"]);
    server.stop().await.unwrap();
}

/// `find` prints the I-Have's object name cleaned in a table and exact in
/// JSON.
#[tokio::test]
async fn find_prints_the_object_name_cleaned_in_a_table() {
    let (mut server, device) = server().await;
    let bbmd = device.to_string();
    let name = hostile("Name");
    let find = |format| {
        let rest = ["--foreign-bbmd", &bbmd, "find", &name, "--wait", "2"];
        let all = args(format, &rest)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        async move {
            let output = support::run(all).await;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        }
    };
    let table = find("table").await;
    assert_clean(&table, &["Name"]);
    let json: Value = serde_json::from_str(&find("json").await).unwrap();
    assert_eq!(json[0]["name"], name, "{json}");
    server.stop().await.unwrap();
}

/// `subscribe` prints a notified string value cleaned in its text lines
/// and exact in its JSON lines. Stopped by SIGTERM, so Unix only.
#[cfg(unix)]
#[tokio::test]
async fn subscribe_prints_notified_text_cleaned() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let (mut server, device) = server().await;
    let target = device.to_string();
    for format in ["table", "json"] {
        let mut child = support::command()
            .args(args(
                format,
                &["subscribe", &target, "characterstring-value:1"],
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        // The initial notification's value line; a JSON line's keys are
        // sorted, so `type` isn't first.
        let wanted = match format {
            "table" => "  PRESENT_VALUE = ",
            _ => "\"type\":\"cov\"",
        };
        let read = tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0, "stdout ended");
                if line.contains(wanted) {
                    return (line, reader);
                }
            }
        });
        // From here a failed assert or a panicking reader kills the child.
        let mut child = KillOnDrop(child);
        let found = tokio::time::timeout(std::time::Duration::from_secs(10), read).await;
        let (line, _reader) = found.expect("no notification line").unwrap();
        if format == "table" {
            assert_clean(&line, &["Value"]);
        } else {
            let json: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                json["values"]["PRESENT_VALUE"],
                format!("\"{}\"", hostile("Value")),
                "{json}"
            );
            assert_eq!(json["decoded"]["PRESENT_VALUE"], hostile("Value"));
        }
        let killed = std::process::Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        let status = support::bounded(async {
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    return status;
                }
                // Process-exit polling only.
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(status.success(), "{status:?}");
    }
    server.stop().await.unwrap();
}

/// A child process killed and reaped when dropped, unless it has exited.
#[cfg(unix)]
struct KillOnDrop(std::process::Child);

#[cfg(unix)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
