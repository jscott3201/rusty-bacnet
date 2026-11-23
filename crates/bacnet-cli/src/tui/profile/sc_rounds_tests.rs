//! `reconnect_max_retries` in an SC profile (#2708): a number of rounds,
//! `0` for no recovery, or the word `"unlimited"`; an absent key is the
//! default of 10. The reconnect keys are refused by their own names, when
//! the profile is read, before anything is dialled.

use super::sc::Rounds;
use super::sc_tests::sc_profile;
use super::store::Store;
use super::*;

/// A saved SC profile named `site` under a fresh root, and its file.
fn saved(p: &Profile) -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    store.save("site", p, true).unwrap();
    let path = root.path().join("profiles/site.toml");
    (root, store, path)
}

/// `path`'s text with its `reconnect_max_retries` line made `line`, or
/// dropped when `line` is `None`.
fn rewrite(path: &std::path::Path, line: Option<&str>) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = String::new();
    for kept in text.lines() {
        match kept.starts_with("reconnect_max_retries") {
            false => out.push_str(kept),
            true => match line {
                Some(line) => out.push_str(line),
                None => continue,
            },
        }
        out.push('\n');
    }
    assert_ne!(out, text);
    std::fs::write(path, out).unwrap();
}

/// A valid SC profile with its identity, as a save leaves it.
fn profile(files: &std::path::Path) -> Profile {
    let mut p = sc_profile(files, "wss://hub.example");
    p.sc_mut().assign_identity().unwrap();
    p
}

#[test]
fn unlimited_recovery_is_written_out_and_reaches_the_client() {
    let files = tempfile::tempdir().unwrap();
    let mut p = profile(files.path());
    p.sc_mut().reconnect_max_retries = Rounds(None);
    let (_root, store, path) = saved(&p);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("reconnect_max_retries = \"unlimited\""),
        "{text}"
    );
    assert_eq!(store.load("site").unwrap(), p);
    let policy = p.transport_args().unwrap().sc_reconnect.unwrap();
    assert_eq!(policy.max_retries, None);
}

/// A file from before #2708 reads as it did: no key is the default of 10,
/// and a number is that many rounds, `0` none.
#[test]
fn numbers_and_an_absent_key_read_as_before() {
    let files = tempfile::tempdir().unwrap();
    let p = profile(files.path());
    let (_root, store, path) = saved(&p);
    for (line, rounds) in [
        (None, Some(10)),
        (Some("reconnect_max_retries = 0"), Some(0)),
        (Some("reconnect_max_retries = 3"), Some(3)),
        (Some("reconnect_max_retries = 4294967295"), Some(u32::MAX)),
    ] {
        rewrite(&path, line);
        let loaded = store.load("site").unwrap();
        let sc = loaded.sc.as_ref().unwrap();
        assert_eq!(sc.reconnect_max_retries, Rounds(rounds), "{line:?}");
        let policy = loaded.transport_args().unwrap().sc_reconnect.unwrap();
        assert_eq!(policy.max_retries, rounds, "{line:?}");
        store.save("site", &loaded, false).unwrap();
    }
}

/// Anything else is refused when the file is read, with the key's line
/// and the values it takes.
#[test]
fn other_values_are_refused_naming_the_key() {
    let files = tempfile::tempdir().unwrap();
    let (_root, store, path) = saved(&profile(files.path()));
    for value in [
        "\"forever\"",
        "\"Unlimited\"",
        "\"10\"",
        "-1",
        "4294967296",
        "1.5",
        "true",
    ] {
        rewrite(&path, Some(&format!("reconnect_max_retries = {value}")));
        let error = store.load("site").unwrap_err();
        assert!(
            error.contains("reconnect_max_retries") && error.contains("\"unlimited\""),
            "{value}: {error}"
        );
    }
}

/// The one-second floor of unlimited recovery and the ten-minute cap are
/// profile errors naming the profile's keys; bounded recovery keeps
/// shorter first delays.
#[test]
fn the_floor_and_the_cap_name_the_profiles_keys() {
    let files = tempfile::tempdir().unwrap();
    let mut p = profile(files.path());
    let sc = p.sc_mut();
    sc.reconnect_initial_delay_ms = 999;
    assert_eq!(p.validate(), Ok(()), "bounded recovery has no floor");
    p.sc_mut().reconnect_max_retries = Rounds(None);
    assert_eq!(
        p.validate(),
        Err("reconnect_initial_delay_ms must be at least 1000 with \
             reconnect_max_retries = \"unlimited\", got 999"
            .into())
    );
    p.sc_mut().reconnect_initial_delay_ms = 1_000;
    assert_eq!(p.validate(), Ok(()));
    p.sc_mut().reconnect_max_delay_ms = 600_001;
    assert_eq!(
        p.validate(),
        Err("reconnect_max_delay_ms must be at most 600000 (10 minutes), got 600001".into())
    );
    p.sc_mut().reconnect_max_delay_ms = 600_000;
    assert_eq!(p.validate(), Ok(()));

    // A hand-edited file is refused when it is read.
    let (_root, store, path) = saved(&p);
    let text = std::fs::read_to_string(&path).unwrap();
    let floor = text.replace(
        "reconnect_initial_delay_ms = 1000",
        "reconnect_initial_delay_ms = 500",
    );
    assert_ne!(floor, text);
    std::fs::write(&path, floor).unwrap();
    let error = store.load("site").unwrap_err();
    assert!(
        error.contains("reconnect_initial_delay_ms must be at least 1000"),
        "{error}"
    );
}

/// The profile refuses exactly what the transport refuses, over every
/// edge of its rules, and always by the profile's key names: a rule the
/// transport gains and the profile lacks fails here, as a disagreement or
/// as the transport's own wording.
#[test]
fn every_reconnect_refusal_matches_the_transport_and_names_profile_keys() {
    let files = tempfile::tempdir().unwrap();
    let mut p = profile(files.path());
    let edges = [0, 1, 999, 1_000, 1_001, 599_999, 600_000, 600_001, u64::MAX];
    for initial in edges {
        for max in edges {
            for rounds in [None, Some(0), Some(1), Some(10), Some(u32::MAX)] {
                let sc = p.sc_mut();
                sc.reconnect_initial_delay_ms = initial;
                sc.reconnect_max_delay_ms = max;
                sc.reconnect_max_retries = Rounds(rounds);
                let transport = p.sc.as_ref().unwrap().reconnect_policy().validate();
                let profile = p.validate();
                let case = format!("{initial} {max} {rounds:?}: {profile:?}");
                assert_eq!(profile.is_ok(), transport.is_ok(), "{case}");
                if let Err(error) = profile {
                    assert!(error.starts_with("reconnect_"), "{case}");
                    assert!(!error.contains("BACnet/SC"), "{case}");
                }
            }
        }
    }
    let sc = p.sc_mut();
    (sc.reconnect_initial_delay_ms, sc.reconnect_max_delay_ms) = (5_000, 4_000);
    assert_eq!(
        p.validate(),
        Err(
            "reconnect_initial_delay_ms must not exceed reconnect_max_delay_ms, \
             got 5000 and 4000"
                .into()
        )
    );
    p.sc_mut().reconnect_max_delay_ms = 0;
    assert_eq!(
        p.validate(),
        Err("reconnect_max_delay_ms must be greater than zero".into())
    );
}

#[test]
fn the_rounds_read_as_words() {
    for (rounds, said) in [
        (Some(0), "no recovery"),
        (Some(1), "1 round"),
        (Some(10), "10 rounds"),
        (None, "unlimited rounds"),
    ] {
        assert_eq!(Rounds(rounds).to_string(), said);
    }
}
