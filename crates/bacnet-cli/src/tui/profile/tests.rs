//! Persistence and resolution regressions use isolated temporary roots.

use super::service::Service;
use super::store::{Store, validate_name};
use super::*;
use clap::{CommandFactory, FromArgMatches};

pub(crate) fn cli(args: &[&str]) -> crate::args::Cli {
    let args: Vec<String> = args.iter().map(|v| (*v).to_owned()).collect();
    crate::parse_on_large_stack(move || {
        let mut matches = crate::args::Cli::command()
            .try_get_matches_from(args)
            .unwrap();
        let explicit_args = crate::args::capture_sources(&matches).unwrap();
        let mut cli = crate::args::Cli::from_arg_matches_mut(&mut matches).unwrap();
        cli.explicit_args = explicit_args;
        cli
    })
}

fn foreign() -> Profile {
    let mut p = Profile::default();
    p.bip_mut().interface = Ipv4Addr::LOCALHOST;
    p.bip_mut().port = 0;
    p.bip_mut().broadcast = Ipv4Addr::new(127, 0, 0, 255);
    p.bip_mut().bbmd = Some("127.0.0.2:47809".parse().unwrap());
    p.bip_mut().ttl = 120;
    p.bip_mut().renewal_secs = Some(30);
    p.safety.writes = Writes::Session;
    p.safety.dangerous = true;
    p
}

#[test]
fn roundtrip_preserves_bip_settings_safety_authorization_and_workspace_isolation() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    assert!(store.list().unwrap().is_empty());
    assert!(
        !store.root().exists(),
        "enumeration must not create persistence"
    );
    let mut profile = foreign();
    profile.authorized_at_unix = Some(1_794_000_000);
    profile.authorized_safety = Some(profile.safety.clone());
    store.save("site-west", &profile, true).unwrap();
    let reopened = Store::new(store.root().into());
    assert_eq!(reopened.load("site-west").unwrap(), profile);
    assert_eq!(reopened.list().unwrap(), ["site-west"]);
    let a = reopened.workspace("site-west").unwrap();
    let b = reopened.workspace("site-east").unwrap();
    assert_ne!(a, b);
    assert!(a.starts_with(store.root()) && b.starts_with(store.root()));
    std::fs::write(a.join("state"), "west").unwrap();
    assert!(!b.join("state").exists());
}

#[test]
fn invalid_names_cannot_escape_or_create_the_root() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        "C:foo",
        "con",
        "COM9",
        "NUL",
        "é",
        "trailing.",
        "a b",
    ] {
        assert!(validate_name(name).is_err(), "{name}");
        assert!(store.load(name).is_err());
        assert!(store.save(name, &Profile::default(), true).is_err());
        assert!(store.workspace(name).is_err());
    }
    assert!(validate_name(&"a".repeat(65)).is_err());
    assert!(!store.root().exists());
}

#[test]
fn create_does_not_overwrite_and_failed_save_keeps_the_prior_file() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let original = foreign();
    store.save("site", &original, true).unwrap();
    assert!(store.save("site", &Profile::default(), true).is_err());
    let mut invalid = original.clone();
    invalid.version = 2;
    assert!(store.save("site", &invalid, false).is_err());
    assert_eq!(store.load("site").unwrap(), original);
    assert_eq!(
        std::fs::read_dir(root.path().join("profiles"))
            .unwrap()
            .count(),
        1,
        "temporary files are cleaned"
    );
}

#[test]
fn malformed_unknown_and_unsupported_profiles_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    store.save("site", &Profile::default(), true).unwrap();
    let path = root.path().join("profiles/site.toml");
    let good = std::fs::read_to_string(&path).unwrap();
    for text in [
        "not = [toml".into(),
        good.replace("version = 1", "version = 2"),
        good.replace("transport = \"bip\"", "transport = \"sc\""),
        good.replace("version = 1", "version = 1\nsecret = 2"),
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(store.load("site").is_err());
    }
}

/// An authorization covers the write policy it was given under: raising
/// the policy, or a file from before the policy was recorded that allows
/// writes, needs authorizing again. Lowering it doesn't.
#[test]
fn raising_the_write_policy_revokes_the_authorization() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    let mut p = foreign();
    p.safety = Safety::READ_ONLY;
    p.authorized_at_unix = Some(1_794_000_000);
    p.authorized_safety = Some(Safety::READ_ONLY);
    store.save("site", &p, true).unwrap();
    assert!(store.load("site").unwrap().authorized_at_unix.is_some());
    for raised in [
        Safety::default(),
        Safety {
            writes: Writes::Session,
            dangerous: true,
        },
    ] {
        p.safety = raised.clone();
        store.save("site", &p, false).unwrap();
        assert_eq!(
            store.load("site").unwrap().authorized_at_unix,
            None,
            "{raised:?}"
        );
    }
    // Authorized under writes = "session": lowering keeps it.
    p.authorized_safety = Some(Safety::default());
    p.safety = Safety::READ_ONLY;
    store.save("site", &p, false).unwrap();
    assert!(store.load("site").unwrap().authorized_at_unix.is_some());
    // A file saved before this PR: authorized, no policy recorded.
    let path = root.path().join("config/profiles/site.toml");
    let old = "version = 1\ntransport = \"bip\"\nauthorized_at_unix = 1794000000\n\n\
               [bip]\n\n[safety]\nwrites = \"session\"\ndangerous = false\n";
    std::fs::write(&path, old).unwrap();
    assert_eq!(store.load("site").unwrap().authorized_at_unix, None);
    std::fs::write(&path, old.replace("\"session\"", "\"never\"")).unwrap();
    assert!(store.load("site").unwrap().authorized_at_unix.is_some());
}

#[test]
fn deterministic_invalid_network_and_safety_settings_fail_before_transport_args() {
    for bbmd in [
        "0.0.0.0:47808",
        "127.0.0.2:0",
        "224.0.0.1:47808",
        "255.255.255.255:47808",
    ] {
        let mut p = foreign();
        p.bip_mut().bbmd = Some(bbmd.parse().unwrap());
        assert!(p.transport_args().is_err(), "{bbmd}");
    }
    let mut broadcast = foreign();
    broadcast.bip_mut().broadcast = "192.0.2.255".parse().unwrap();
    broadcast.bip_mut().bbmd = Some("192.0.2.255:47808".parse().unwrap());
    assert!(broadcast.transport_args().is_err());
    // The transport permits a loopback broadcast IP, or the interface IP,
    // as a BBMD at another port. Equality with the bound endpoint is later.
    let mut loopback = foreign();
    loopback.bip_mut().broadcast = Ipv4Addr::LOCALHOST;
    loopback.bip_mut().bbmd = Some("127.0.0.1:47809".parse().unwrap());
    loopback.transport_args().unwrap();
    for (ttl, renewal) in [(0, None), (2, Some(0)), (2, Some(2)), (2, Some(3))] {
        let mut p = foreign();
        p.bip_mut().ttl = ttl;
        p.bip_mut().renewal_secs = renewal;
        assert!(p.transport_args().is_err());
    }
    let mut p = Profile::default();
    p.safety.dangerous = true;
    p.transport_args().unwrap();
    p.safety.writes = Writes::Never;
    assert!(
        p.transport_args().is_err(),
        "dangerous needs write sessions"
    );
    p.safety.dangerous = false;
    p.bip_mut().min_interval_ms = 3_600_001;
    assert!(p.transport_args().is_err());
    p.bip_mut().min_interval_ms = 0;
    p.bip_mut().port = 0;
    p.transport_args().unwrap();
}

#[test]
fn explicit_default_values_override_saved_settings_on_either_side_of_tui() {
    let mut p = foreign();
    p.bip_mut().port = 47809;
    p.bip_mut().timeout_ms = 42;
    p.bip_mut().min_interval_ms = 123;
    let cli = cli(&[
        "bacnet",
        "--port",
        "47808",
        "--broadcast",
        "255.255.255.255",
        "tui",
        "--profile",
        "site",
        "--timeout",
        "6000",
        "--min-interval-ms",
        "0",
        "--ttl",
        "300",
    ]);
    let resolved = Overrides::from_cli(&cli).unwrap().resolve(&p).unwrap();
    assert_eq!(resolved.bip().port, 47808);
    assert_eq!(resolved.bip().broadcast, Ipv4Addr::BROADCAST);
    assert_eq!(resolved.bip().timeout_ms, 6000);
    assert_eq!(resolved.bip().min_interval_ms, 0);
    assert_eq!(resolved.bip().ttl, 300);
    assert_eq!(resolved.bip().bbmd, p.bip().bbmd);
    assert_eq!(resolved.bip().interface, p.bip().interface);
}

#[test]
fn absent_flags_keep_profile_values_and_renewal_can_use_its_bbmd() {
    let p = foreign();
    let absent = cli(&["bacnet", "tui", "--profile", "site"]);
    assert_eq!(
        Overrides::from_cli(&absent).unwrap().resolve(&p).unwrap(),
        p
    );
    let renewal = cli(&["bacnet", "tui", "--profile", "site", "--renewal", "12"]);
    assert_eq!(
        Overrides::from_cli(&renewal)
            .unwrap()
            .resolve(&p)
            .unwrap()
            .bip()
            .renewal_secs,
        Some(12)
    );
    let ipv6 = cli(&["bacnet", "tui", "--profile", "site", "--ipv6"]);
    assert!(Overrides::from_cli(&ipv6).is_err());
}

#[test]
fn first_use_requires_attestation_and_saves_it_before_preparing_a_workspace() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = foreign();
    store.save("site", &p, true).unwrap();
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::default(),
    };
    assert!(service.prepare("site", &p, false, &[]).is_err());
    assert!(store.load("site").unwrap().authorized_at_unix.is_none());
    assert!(!root.path().join("workspaces").exists());
    let resolved = service.prepare("site", &p, true, &[]).unwrap();
    let persisted = store.load("site").unwrap();
    assert!(persisted.authorized_at_unix.is_some());
    assert_eq!(
        resolved.resolved.authorized_at_unix,
        persisted.authorized_at_unix
    );
    assert!(root.path().join("workspaces/site").is_dir());
    assert_eq!(
        service
            .prepare("site", &persisted, false, &[])
            .unwrap()
            .resolved,
        persisted
    );
}

#[test]
fn stale_preview_and_stale_edits_never_overwrite_or_connect() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let original = foreign();
    store.save("site", &original, true).unwrap();
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::default(),
    };
    let mut changed = original.clone();
    changed.bip_mut().port = 1234;
    store.save("site", &changed, false).unwrap();
    assert!(
        service
            .prepare("site", &original, true, &[])
            .unwrap_err()
            .contains("changed on disk")
    );
    assert!(
        service
            .save("site", original.clone(), false, Some(original))
            .is_err()
    );
    assert_eq!(store.load("site").unwrap(), changed);
}

#[test]
fn workspace_failure_does_not_produce_a_prepared_connection() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = foreign();
    store.save("site", &p, true).unwrap();
    std::fs::write(root.path().join("workspaces"), "blocked").unwrap();
    let service = Service {
        store: Ok(store),
        overrides: Overrides::default(),
    };
    assert!(
        service
            .prepare("site", &p, true, &[])
            .unwrap_err()
            .contains("workspace")
    );
}

#[cfg(unix)]
#[test]
fn failed_authorization_save_preserves_the_document_and_prevents_preparation() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = foreign();
    store.save("site", &p, true).unwrap();
    let directory = root.path().join("profiles");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::default(),
    };
    let result = service.prepare("site", &p, true, &[]);
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        result.is_err(),
        "authorization must not be usable when its save fails"
    );
    assert_eq!(store.load("site").unwrap(), p);
    assert!(!root.path().join("workspaces").exists());
}

#[test]
fn startup_zero_one_many_and_explicit_selection_are_predictable() {
    use crate::tui::app::connection::Screen;
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::default(),
    };
    let empty = startup::select(&service, None);
    assert_eq!(empty.screen, Screen::Edit);
    assert!(empty.creating);
    assert!(!root.path().join("profiles").exists());
    store.save("alpha", &Profile::default(), true).unwrap();
    let one = startup::select(&service, None);
    assert_eq!(one.screen, Screen::Preview);
    assert_eq!(one.name, "alpha");
    store.save("beta", &foreign(), true).unwrap();
    assert_eq!(startup::select(&service, None).screen, Screen::List);
    assert_eq!(startup::select(&service, Some("beta")).name, "beta");
    assert!(startup::select(&service, Some("missing")).error.is_some());
}

#[test]
fn flags_only_entry_skips_profile_discovery_while_named_profiles_accept_overrides() {
    for args in [
        vec!["bacnet", "tui", "--no-profile"],
        vec!["bacnet", "--port", "47808", "tui"],
        vec!["bacnet", "tui", "--ipv6"],
        vec!["bacnet", "--sc", "tui"],
    ] {
        let cli = cli(&args);
        assert!(!startup::enabled(&cli));
        let opened = startup::open(&cli, || {
            panic!("a flags-only run must not discover a store")
        });
        assert!(opened.unwrap().is_none());
    }
    assert!(startup::enabled(&cli(&["bacnet", "tui"])));
    assert!(startup::enabled(&cli(&[
        "bacnet",
        "--port",
        "0",
        "tui",
        "--profile",
        "site"
    ])));
}

#[test]
fn enumeration_errors_are_visible_without_preventing_named_profile_reads() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("profiles"), "not a directory").unwrap();
    let service = Service {
        store: Ok(Store::new(root.path().into())),
        overrides: Overrides::default(),
    };
    let initial = startup::select(&service, None);
    assert!(initial.error.unwrap().contains("list profiles"));
    assert!(!root.path().join("workspaces").exists());
}

#[test]
fn first_attestation_persists_raw_profile_without_transient_cli_overrides() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into());
    let p = foreign();
    store.save("site", &p, true).unwrap();
    let cli = cli(&[
        "bacnet",
        "tui",
        "--profile",
        "site",
        "--port",
        "1234",
        "--timeout",
        "6000",
        "--renewal",
        "12",
    ]);
    let service = Service {
        store: Ok(store.clone()),
        overrides: Overrides::from_cli(&cli).unwrap(),
    };
    let effective = service.prepare("site", &p, true, &[]).unwrap();
    assert_eq!(effective.resolved.bip().port, 1234);
    assert_eq!(effective.resolved.bip().renewal_secs, Some(12));
    let stored = store.load("site").unwrap();
    assert_eq!(stored.bip, p.bip);
    assert_eq!(stored.safety, p.safety);
    assert!(stored.authorized_at_unix.is_some());
    assert_eq!(
        stored.authorized_at_unix,
        effective.resolved.authorized_at_unix
    );
}

#[test]
fn explicit_broadcast_survives_interface_autoselection_and_picker_choices() {
    let cli = cli(&["bacnet", "tui", "--broadcast", "255.255.255.255"]);
    let (_, broadcast, picker) = crate::tui::choose_interface(
        &cli,
        crate::tui::worker::session::TransportKind::Bip,
        &crate::tui::log_layer::LogRing::new(16),
    );
    assert_eq!(broadcast, Ipv4Addr::BROADCAST);
    if let Some(picker) = picker {
        assert!(
            picker
                .items
                .iter()
                .all(|item| item.broadcast == Ipv4Addr::BROADCAST)
        );
    }
}
