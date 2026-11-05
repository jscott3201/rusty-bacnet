//! A B/IP profile's `share_port` (#2366): saved only when on, handed to the
//! transport, and set from `--share-port` with a named profile.

use super::overrides::{self, Overrides};
use super::store::Store;
use super::tests::cli;
use super::*;

#[test]
fn a_profiles_share_port_reaches_the_args_and_saves_only_when_on() {
    assert!(!Profile::default().transport_args().unwrap().share_port);
    let mut profile = Profile::default();
    profile.bip_mut().share_port = true;
    assert!(profile.transport_args().unwrap().share_port);
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    store.save("shared", &profile, true).unwrap();
    let text = std::fs::read_to_string(root.path().join("config/profiles/shared.toml")).unwrap();
    assert!(text.contains("share_port = true"), "{text}");
    assert_eq!(store.load("shared").unwrap(), profile);
    // Off, the field stays out of the file, so older profiles save
    // unchanged.
    store.save("plain", &Profile::default(), true).unwrap();
    let text = std::fs::read_to_string(root.path().join("config/profiles/plain.toml")).unwrap();
    assert!(!text.contains("share_port"), "{text}");
}

#[test]
fn the_flag_turns_sharing_on_for_a_named_b_ip_profile() {
    let flag = cli(&["bacnet", "tui", "--profile", "site", "--share-port"]);
    assert!(overrides::has_transport_flags(&flag));
    let resolved = Overrides::from_cli(&flag)
        .unwrap()
        .resolve(&Profile::default())
        .unwrap();
    assert!(resolved.bip().share_port);
    assert!(resolved.transport_args().unwrap().share_port);
    // Without the flag the profile's own choice stays.
    let absent = cli(&["bacnet", "tui", "--profile", "site"]);
    assert!(!overrides::has_transport_flags(&absent));
    let mut shared = Profile::default();
    shared.bip_mut().share_port = true;
    let resolved = Overrides::from_cli(&absent)
        .unwrap()
        .resolve(&shared)
        .unwrap();
    assert_eq!(resolved, shared);
    // An SC profile takes no B/IP flag.
    let refused = Overrides::from_cli(&flag)
        .unwrap()
        .resolve(&Profile::sc_default());
    assert!(refused.is_err(), "{refused:?}");
}
