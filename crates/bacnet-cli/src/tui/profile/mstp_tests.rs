//! MS/TP profiles (#969): the `[mstp]` table, its checks, the connection
//! settings it resolves to, and the flags that override it.

use super::store::Store;
use super::tests::cli;
use super::*;
use crate::tui::app::connection::{Connection, Key, Screen};

/// An MS/TP profile as written to disk, byte for byte.
const MSTP_FILE: &str = r#"version = 1
transport = "mstp"

[mstp]
port = "/dev/ttyUSB0"
mac = 3
baud = 76800
max_master = 32
max_info_frames = 2
timeout_ms = 6000
min_interval_ms = 0

[safety]
writes = "session"
dangerous = false
"#;

fn mstp_profile() -> Profile {
    let mut p = Profile::mstp_default();
    let m = p.mstp_mut();
    m.port = "/dev/ttyUSB0".into();
    m.mac = 3;
    m.baud = 76_800;
    m.max_master = 32;
    m.max_info_frames = 2;
    p
}

#[test]
fn an_mstp_profile_saves_its_table_and_loads_it_back() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    store.save("trunk", &mstp_profile(), true).unwrap();
    let path = root.path().join("profiles/trunk.toml");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), MSTP_FILE);
    assert_eq!(store.load("trunk").unwrap(), mstp_profile());

    // The defaults fill in what a hand-written file leaves out; the port
    // and MAC are required, and the table must match the transport.
    let short = "version = 1\ntransport = \"mstp\"\n\n[mstp]\nport = \"COM3\"\nmac = 0\n\n\
                 [safety]\nwrites = \"never\"\ndangerous = false\n";
    std::fs::write(&path, short).unwrap();
    let m = store.load("trunk").unwrap().mstp.unwrap();
    assert_eq!(
        (m.baud, m.max_master, m.max_info_frames, m.timeout_ms),
        (38_400, 127, 1, 6000)
    );
    for text in [
        MSTP_FILE.replace("mac = 3\n", ""),
        MSTP_FILE.replace("port = \"/dev/ttyUSB0\"\n", ""),
        MSTP_FILE.replace("[mstp]", "[bip]"),
        MSTP_FILE.replace("transport = \"mstp\"", "transport = \"bip\""),
        MSTP_FILE.replace("[mstp]\n", "[mstp]\nparity = \"none\"\n"),
    ] {
        std::fs::write(&path, &text).unwrap();
        assert!(store.load("trunk").is_err(), "{text}");
    }
}

#[test]
fn mstp_settings_are_checked_before_any_port_opens() {
    assert_eq!(mstp_profile().validate(), Ok(()));
    for (change, expected) in [
        (
            (|m: &mut Mstp| m.baud = 4800) as fn(&mut Mstp),
            "baud must be one of",
        ),
        (
            |m| m.mac = 128,
            "mac a master station's MAC must be 0 to 127",
        ),
        (|m| m.mac = 40, "mac 40 is above max_master 32"),
        (
            |m| m.max_info_frames = 0,
            "max_info_frames must be 1 to 255",
        ),
        (|m| m.port.clear(), "serial port is required"),
        (|m| m.min_interval_ms = 3_600_001, "min_interval_ms"),
    ] {
        let mut p = mstp_profile();
        change(p.mstp_mut());
        let error = p.validate().unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(p.transport_args().is_err());
    }
    let args = mstp_profile().transport_args().unwrap();
    assert_eq!(args.mstp, Some(mstp_profile().mstp.unwrap().args()));
    assert_eq!(args.timeout_ms, 6000);
    assert!(!args.sc && args.foreign_device.is_none());
    assert_eq!(Transport::Mstp.kind().label(), "MSTP");
}

/// The MS/TP flags override the table; another transport's flags, or the
/// MS/TP flags on another profile, are refused rather than dropped.
#[test]
fn mstp_flags_override_an_mstp_profile_only() {
    let overrides = Overrides::from_cli(&cli(&[
        "bacnet",
        "--mstp",
        "/dev/ttyUSB1",
        "--mstp-mac",
        "5",
        "--timeout",
        "9000",
        "tui",
        "--profile",
        "trunk",
    ]))
    .unwrap();
    let resolved = overrides.resolve(&mstp_profile()).unwrap();
    let m = resolved.mstp.unwrap();
    assert_eq!(
        (m.port.as_str(), m.mac, m.baud, m.timeout_ms),
        ("/dev/ttyUSB1", 5, 76_800, 9000)
    );
    let error = overrides.resolve(&Profile::default()).unwrap_err();
    assert!(error.contains("MS/TP flags"), "{error}");
    let bip_flags =
        Overrides::from_cli(&cli(&["bacnet", "-p", "47809", "tui", "--profile", "x"])).unwrap();
    let error = bip_flags.resolve(&mstp_profile()).unwrap_err();
    assert!(error.contains("don't apply to an MS/TP profile"), "{error}");
    // `--mstp` without --profile is a flags-only session.
    assert!(!startup::enabled(&cli(&[
        "bacnet",
        "--mstp",
        "/dev/ttyUSB0",
        "--mstp-mac",
        "3",
        "tui"
    ])));
}

/// The editor makes an MS/TP profile; its MAC starts blank, a choice.
#[test]
fn the_editor_creates_an_mstp_profile() {
    let mut connection = Connection::new(Overrides::default());
    assert_eq!(connection.key(Key::NewMstp), None);
    assert_eq!(connection.screen, Screen::Edit);
    assert_eq!(connection.transport, Transport::Mstp);
    assert_eq!(connection.labels().len(), connection.fields.len());
    assert_eq!(connection.fields[2], "", "no default MAC");
    for ch in "trunk".chars() {
        connection.key(Key::Char(ch));
    }
    connection.key(Key::Next);
    for ch in "/dev/ttyUSB0".chars() {
        connection.key(Key::Char(ch));
    }
    assert_eq!(connection.key(Key::Submit), None);
    assert!(
        connection
            .error
            .as_ref()
            .unwrap()
            .contains("MAC must be 0 to 127")
    );
    connection.key(Key::Next);
    connection.key(Key::Char('3'));
    let Some(crate::tui::message::Command::Profile(service::Request::Save { profile, .. })) =
        connection.key(Key::Submit)
    else {
        panic!("expected a save: {:?}", connection.error);
    };
    let m = profile.mstp.unwrap();
    assert_eq!(
        (m.port.as_str(), m.mac, m.baud),
        ("/dev/ttyUSB0", 3, 38_400)
    );
}
