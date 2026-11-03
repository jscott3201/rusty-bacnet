//! A B/IP profile's NAT global address (#1943): validated, saved, set from
//! `--nat-global-address`, and handed to the Network screen only.

use super::overrides::Overrides;
use super::store::Store;
use super::tests::cli;
use super::*;

fn behind_nat() -> Profile {
    let mut p = Profile::default();
    p.bip_mut().bbmd = Some("198.51.100.1:47808".parse().unwrap());
    p.bip_mut().nat_global_address = Some("203.0.113.7:61000".parse().unwrap());
    p
}

#[test]
fn a_profiles_global_address_survives_a_save_and_reaches_the_args() {
    let profile = behind_nat();
    let args = profile.transport_args().unwrap();
    assert_eq!(
        args.nat_global_address,
        Some("203.0.113.7:61000".parse().unwrap())
    );
    // A display hint only: the registration itself is unchanged.
    let foreign = args.foreign_device.unwrap();
    assert_eq!(foreign.bbmd, (Ipv4Addr::new(198, 51, 100, 1), 47808).into());
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    store.save("nat", &profile, true).unwrap();
    let text = std::fs::read_to_string(root.path().join("config/profiles/nat.toml")).unwrap();
    assert!(
        text.contains("nat_global_address = \"203.0.113.7:61000\""),
        "{text}"
    );
    assert_eq!(store.load("nat").unwrap(), profile);
    // Unset, the field stays out of the file, so older profiles save
    // unchanged.
    store.save("plain", &Profile::default(), true).unwrap();
    let text = std::fs::read_to_string(root.path().join("config/profiles/plain.toml")).unwrap();
    assert!(!text.contains("nat_global_address"), "{text}");
}

#[test]
fn a_profiles_global_address_is_refused_where_it_cant_apply() {
    let refused = |edit: fn(&mut Bip), why: &str| {
        let mut p = behind_nat();
        edit(p.bip_mut());
        let error = p.validate().unwrap_err();
        assert!(error.contains(why), "{error}");
    };
    refused(|b| b.bbmd = None, "nat_global_address requires a BBMD");
    for (global, why) in [
        ("203.0.113.7:0", "nonzero port"),
        ("0.0.0.0:61000", "unicast"),
        ("239.1.2.3:61000", "unicast"),
        ("255.255.255.255:61000", "unicast"),
    ] {
        let mut p = behind_nat();
        p.bip_mut().nat_global_address = Some(global.parse().unwrap());
        let error = p.validate().unwrap_err();
        assert!(error.contains(why), "{global}: {error}");
    }
}

#[test]
fn the_flag_overrides_a_profiles_global_address() {
    let p = behind_nat();
    let flag = cli(&[
        "bacnet",
        "tui",
        "--profile",
        "site",
        "--nat-global-address",
        "203.0.113.8:62000",
    ]);
    let resolved = Overrides::from_cli(&flag).unwrap().resolve(&p).unwrap();
    assert_eq!(
        resolved.bip().nat_global_address,
        Some("203.0.113.8:62000".parse().unwrap())
    );
    assert!(overrides::has_transport_flags(&flag));
    // Without the flag, the profile's own address stays.
    let absent = cli(&["bacnet", "tui", "--profile", "site"]);
    let resolved = Overrides::from_cli(&absent).unwrap().resolve(&p).unwrap();
    assert_eq!(resolved, p);
    // An SC profile takes no B/IP flag.
    assert!(
        Overrides::from_cli(&flag)
            .unwrap()
            .resolve(&Profile::sc_default())
            .is_err()
    );
}
