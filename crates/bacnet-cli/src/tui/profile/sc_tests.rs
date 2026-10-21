//! BACnet/SC profiles: schema, identity persistence, overrides, and what a
//! saved file may contain. Each test uses its own temporary config root.

use super::service::{Reply, Service};
use super::store::Store;
use super::*;
use std::path::{Path, PathBuf};

fn cli(args: &[&str]) -> crate::args::Cli {
    super::tests::cli(args)
}

/// Real PEM files: a self-signed certificate, used as CA and node cert, and
/// its private key. Returns (ca, cert, key) paths and the key's PEM text.
pub(crate) fn pem_files(dir: &Path) -> (PathBuf, PathBuf, PathBuf, String) {
    let pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])
        .unwrap()
        .self_signed(&pair)
        .unwrap();
    let (ca, crt, key) = (
        dir.join("ca.pem"),
        dir.join("node.pem"),
        dir.join("node.key"),
    );
    std::fs::write(&ca, cert.pem()).unwrap();
    std::fs::write(&crt, cert.pem()).unwrap();
    std::fs::write(&key, pair.serialize_pem()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    (ca, crt, key, pair.serialize_pem())
}

/// An SC profile naming `dir`'s files, with no identity yet.
pub(crate) fn sc_profile(dir: &Path, hub: &str) -> Profile {
    let (ca, cert, key, _) = pem_files(dir);
    let mut p = Profile::sc_default();
    let sc = p.sc_mut();
    sc.hub_url = hub.into();
    sc.ca = ca;
    sc.cert = cert;
    sc.key = key;
    p
}

fn service(root: &Path, overrides: Overrides) -> Service {
    Service::new(Ok(Store::new(root.to_owned())), overrides)
}

fn uuid_of(p: &Profile) -> sc::DeviceUuid {
    p.sc.as_ref().unwrap().device_uuid.expect("assigned")
}

/// A B/IP file written before SC support existed, byte for byte.
const BIP_FILE_BEFORE_SC: &str = r#"version = 1
transport = "bip"
authorized_at_unix = 1794000000

[bip]
interface = "127.0.0.1"
port = 0
broadcast = "127.0.0.255"
timeout_ms = 6000
min_interval_ms = 0
bbmd = "127.0.0.2:47809"
ttl = 120
renewal_secs = 30

[safety]
writes = "session"
dangerous = true

[authorized_safety]
writes = "session"
dangerous = true
"#;

#[test]
fn bip_profiles_written_before_sc_support_load_and_save_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("old.toml"), BIP_FILE_BEFORE_SC).unwrap();
    let store = Store::new(root.path().into());
    let p = store.load("old").unwrap();
    assert_eq!(p.transport, Transport::Bip);
    assert_eq!(p.sc, None);
    assert_eq!(p.bip().bbmd, Some("127.0.0.2:47809".parse().unwrap()));
    assert_eq!(p.authorized_at_unix, Some(1_794_000_000));
    store.save("old", &p, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("old.toml")).unwrap(),
        BIP_FILE_BEFORE_SC
    );
}

#[test]
fn transport_tables_must_match_and_unknown_sc_keys_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let path = root.path().join("profiles/site.toml");
    let dir = tempfile::tempdir().unwrap();
    let mut p = sc_profile(dir.path(), "wss://hub.example:443");
    p.sc_mut().assign_identity().unwrap();
    store.save("site", &p, true).unwrap();
    let good = std::fs::read_to_string(&path).unwrap();
    assert!(good.contains("transport = \"sc\"") && good.contains("[sc]"));
    assert!(!good.contains("[bip]"));
    for text in [
        good.replace("transport = \"sc\"", "transport = \"bip\""),
        good.replace("[sc]", "[bip]"),
        good.replace("[sc]\n", "[sc]\nkey_pem = \"x\"\n"),
        good.replace("[sc]\n", "[sc]\npassword = \"x\"\n"),
        good.replace("wss://hub.example:443", "ws://hub.example:443"),
    ] {
        std::fs::write(&path, &text).unwrap();
        assert!(store.load("site").is_err(), "{text}");
    }
}

#[test]
fn serialized_sc_profile_holds_paths_and_never_pem_or_key_bytes() {
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (ca, cert, key, key_pem) = pem_files(files.path());
    let mut p = Profile::sc_default();
    let sc = p.sc_mut();
    sc.hub_url = "wss://hub.example:443/sc".into();
    sc.failover_hub_url = Some("wss://hub2.example:443/sc".into());
    (sc.ca, sc.cert, sc.key) = (ca.clone(), cert.clone(), key.clone());
    service(root.path(), Overrides::default())
        .save("site", p, true, None)
        .unwrap();
    let text = std::fs::read_to_string(root.path().join("profiles/site.toml")).unwrap();
    let table: toml::Table = text.parse().unwrap();
    let saved = table["sc"].as_table().unwrap();
    for (field, path) in [("ca", &ca), ("cert", &cert), ("key", &key)] {
        assert_eq!(saved[field].as_str(), path.to_str(), "{field} in\n{text}");
    }
    assert!(!text.contains("-----"), "PEM armour in\n{text}");
    assert!(!text.contains("PRIVATE"));
    // No run of the key's base64 body, however short, made it in.
    let body: String = key_pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    for window in body.as_bytes().chunks(12).filter(|c| c.len() == 12) {
        let window = std::str::from_utf8(window).unwrap();
        assert!(!text.contains(window), "key bytes {window} in\n{text}");
    }
    let mut keys: Vec<_> = saved.keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "ca",
            "cert",
            "connect_timeout_ms",
            "device_uuid",
            "failover_hub_url",
            "hub_url",
            "key",
            "min_interval_ms",
            "reconnect",
            "reconnect_initial_delay_ms",
            "reconnect_max_delay_ms",
            "reconnect_max_retries",
            "timeout_ms",
            "vmac"
        ]
    );
}

#[test]
fn sc_device_uuid_is_generated_once_and_identical_across_three_restarts() {
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let p = sc_profile(files.path(), "wss://127.0.0.1:47808");
    let Reply::Loaded { profile: saved, .. } = service(root.path(), Overrides::default())
        .save("site", p, true, None)
        .unwrap()
    else {
        panic!("save replies with the profile");
    };
    let uuid = uuid_of(&saved);
    let vmac = saved.sc.as_ref().unwrap().vmac.unwrap();
    for restart in 0..3 {
        // A fresh service and store over the same directory: a new process.
        let service = service(root.path(), Overrides::default());
        let Reply::Loaded { profile, .. } = service.load("site").unwrap() else {
            panic!("load replies with the profile");
        };
        assert_eq!(uuid_of(&profile), uuid, "restart {restart}");
        let prepared = service.prepare("site", &profile, true, &[]).unwrap();
        assert_eq!(uuid_of(&prepared.resolved), uuid);
        assert_eq!(prepared.resolved.sc.as_ref().unwrap().vmac, Some(vmac));
        let args = prepared.resolved.transport_args().unwrap();
        assert_eq!(args.sc_device_uuid, Some(uuid.0));
    }
    // Saving an edit keeps the identity too.
    let service = service(root.path(), Overrides::default());
    let stored = Store::new(root.path().into()).load("site").unwrap();
    let mut edited = stored.clone();
    edited.sc_mut().timeout_ms = 9000;
    let Reply::Loaded { profile, .. } = service.save("site", edited, false, Some(stored)).unwrap()
    else {
        panic!()
    };
    assert_eq!(uuid_of(&profile), uuid);
}

#[test]
fn a_hand_written_sc_profile_gets_its_identity_on_first_connect_only() {
    let root = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = sc_profile(files.path(), "wss://127.0.0.1:47808");
    store.save("site", &p, true).unwrap();
    let service = service(root.path(), Overrides::default());
    // Previewing writes nothing.
    service.load("site").unwrap();
    assert_eq!(store.load("site").unwrap(), p);
    // Refused for lack of authorization: no identity is stored either.
    assert!(service.prepare("site", &p, false, &[]).is_err());
    assert_eq!(store.load("site").unwrap().sc.unwrap().device_uuid, None);
    let first = service.prepare("site", &p, true, &[]).unwrap();
    assert_eq!(first.stored, store.load("site").unwrap());
    let uuid = uuid_of(&first.stored);
    let again = service.prepare("site", &first.stored, false, &[]).unwrap();
    assert_eq!(uuid_of(&again.resolved), uuid);
}

#[test]
fn sc_flags_override_profile_fields_and_flags_for_the_other_transport_are_refused() {
    let files = tempfile::tempdir().unwrap();
    let mut p = sc_profile(files.path(), "wss://saved.example:443");
    p.sc_mut().assign_identity().unwrap();
    p.sc_mut().timeout_ms = 9000;
    let flags = cli(&[
        "bacnet",
        "--sc-url",
        "wss://flag.example:8443",
        "--sc-ca",
        "relative-ca.pem",
        "--sc-device-uuid",
        "00112233-4455-6677-8899-aabbccddeeff",
        "--timeout",
        "6000",
        "tui",
        "--profile",
        "site",
    ]);
    let resolved = Overrides::from_cli(&flags).unwrap().resolve(&p).unwrap();
    let (saved, effective) = (p.sc.as_ref().unwrap(), resolved.sc.as_ref().unwrap());
    assert_eq!(effective.hub_url, "wss://flag.example:8443");
    assert!(effective.ca.is_absolute() && effective.ca.ends_with("relative-ca.pem"));
    assert_eq!(effective.cert, saved.cert);
    assert_eq!(
        effective.device_uuid.unwrap().to_string(),
        "00112233-4455-6677-8899-aabbccddeeff"
    );
    assert_eq!(effective.vmac, saved.vmac);
    assert_eq!(effective.timeout_ms, 6000);
    // Absent flags keep every saved value, defaults included.
    let none = Overrides::from_cli(&cli(&["bacnet", "tui", "--profile", "site"])).unwrap();
    assert_eq!(none.resolve(&p).unwrap(), p);
    let bip_flag = cli(&["bacnet", "--port", "47809", "tui", "--profile", "site"]);
    assert!(Overrides::from_cli(&bip_flag)
        .unwrap()
        .resolve(&p)
        .unwrap_err()
        .contains("SC profile"));
    let sc_flag = cli(&["bacnet", "--sc", "tui", "--profile", "site"]);
    assert!(Overrides::from_cli(&sc_flag)
        .unwrap()
        .resolve(&Profile::default())
        .unwrap_err()
        .contains("B/IP profile"));
    assert!(Overrides::from_cli(&sc_flag).unwrap().resolve(&p).is_ok());
}

/// One way to break an otherwise valid SC table.
type Break = fn(&mut Sc);

#[test]
fn sc_validation_covers_urls_paths_and_failover() {
    let files = tempfile::tempdir().unwrap();
    let base = sc_profile(files.path(), "wss://hub.example");
    assert_eq!(base.validate(), Ok(()));
    let cases: [(Break, &str); 9] = [
        (|s| s.hub_url = "https://hub".into(), "wss://"),
        (
            |s| {
                s.failover_hub_url = Some("wss://hub2.example".into());
                s.reconnect = false;
            },
            "needs reconnect on",
        ),
        (
            |s| s.connect_timeout_ms = 0,
            "connect_timeout_ms must be 1 to",
        ),
        (|s| s.reconnect_initial_delay_ms = 0, "reconnect settings"),
        (
            |s| s.reconnect_initial_delay_ms = s.reconnect_max_delay_ms + 1,
            "must not exceed",
        ),
        (|s| s.hub_url = "wss://hub:0".into(), "1-65535"),
        (
            |s| s.failover_hub_url = Some("WSS://HUB.example/".into()),
            "differ",
        ),
        (|s| s.failover_hub_url = Some("hub2".into()), "Failover URL"),
        (|s| s.key = PathBuf::new(), "Key file"),
    ];
    for (break_it, why) in cases {
        let mut p = base.clone();
        break_it(p.sc_mut());
        let error = p.validate().unwrap_err();
        assert!(error.contains(why), "{error}");
    }
    // Without an identity there is nothing to connect as.
    assert!(base.transport_args().err().unwrap().contains("UUID"));
}

/// #1831: a primary-only profile reconnects by default, with the client's
/// own backoff and connect timeout; the profile can turn reconnect off and
/// tune both, and what it sets reaches the client's settings.
#[test]
fn sc_reconnect_and_connect_timeout_reach_the_transport_settings() {
    let files = tempfile::tempdir().unwrap();
    let mut p = sc_profile(files.path(), "wss://hub.example");
    p.sc_mut().assign_identity().unwrap();
    // A file written before these keys existed loads with the defaults.
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    store.save("site", &p, true).unwrap();
    let path = root.path().join("profiles/site.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let old: String = text
        .lines()
        .filter(|l| !l.starts_with("reconnect") && !l.starts_with("connect_timeout_ms"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_ne!(old, text);
    std::fs::write(&path, old).unwrap();
    assert_eq!(store.load("site").unwrap(), p);

    let args = p.transport_args().unwrap();
    let policy = args
        .sc_reconnect
        .expect("primary-only reconnects by default");
    let default = bacnet_transport::sc::ScReconnectConfig::default();
    assert_eq!(
        (
            policy.initial_delay_ms,
            policy.max_delay_ms,
            policy.max_retries
        ),
        (
            default.initial_delay_ms,
            default.max_delay_ms,
            default.max_retries
        )
    );
    assert_eq!(args.sc_connect_timeout_ms, Some(10_000));
    assert_eq!(args.sc_failover_url, None);

    let sc = p.sc_mut();
    (sc.reconnect_initial_delay_ms, sc.reconnect_max_delay_ms) = (500, 4_000);
    (sc.reconnect_max_retries, sc.connect_timeout_ms) = (3, 2_500);
    let args = p.transport_args().unwrap();
    let policy = args.sc_reconnect.unwrap();
    assert_eq!(
        (
            policy.initial_delay_ms,
            policy.max_delay_ms,
            policy.max_retries
        ),
        (500, 4_000, 3)
    );
    assert_eq!(args.sc_connect_timeout_ms, Some(2_500));
    p.sc_mut().reconnect = false;
    assert!(p.transport_args().unwrap().sc_reconnect.is_none(), "off");
}

/// The SC flags override a profile like the others: a failover hub (which
/// turns reconnect back on), `--sc-reconnect`, and the connect timeout.
#[test]
fn sc_failover_reconnect_and_timeout_flags_override_a_profile() {
    let files = tempfile::tempdir().unwrap();
    let mut p = sc_profile(files.path(), "wss://hub.example");
    p.sc_mut().assign_identity().unwrap();
    p.sc_mut().reconnect = false;
    let resolve = |flags: &[&str]| {
        let mut args = vec!["bacnet"];
        args.extend_from_slice(flags);
        args.extend(["tui", "--profile", "site"]);
        Overrides::from_cli(&cli(&args)).unwrap().resolve(&p)
    };
    let effective = resolve(&[
        "--sc-failover-url",
        "wss://hub2.example",
        "--sc-connect-timeout",
        "3000",
    ])
    .unwrap();
    let sc = effective.sc.unwrap();
    assert_eq!(sc.failover_hub_url.as_deref(), Some("wss://hub2.example"));
    assert!(sc.reconnect, "a failover hub brings reconnect");
    assert_eq!(sc.connect_timeout_ms, 3000);
    assert!(resolve(&["--sc-reconnect"]).unwrap().sc.unwrap().reconnect);
    assert!(!resolve(&[]).unwrap().sc.unwrap().reconnect, "kept off");
    let error = resolve(&["--sc-failover-url", "wss://hub.example"]).unwrap_err();
    assert!(error.contains("differ"), "{error}");
}
