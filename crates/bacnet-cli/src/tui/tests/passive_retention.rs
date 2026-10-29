//! Bounded observation state under repeated and high-cardinality input.
use super::*;
use crate::tui::app::who_has::{MAX_OBSERVATION_BYTES, MAX_OBSERVATIONS};
use crate::tui::network::{
    discovery::{DiscoveryEvent, FoundRow},
    routers::{MAX_ROUTER_NETWORKS, MAX_ROUTERS, RouterMap},
};
use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};
fn found(address: usize, name: String) -> FoundRow {
    FoundRow {
        device: 12,
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        name,
        address: format!("station-{address:05}"),
        via: None,
        network: None,
    }
}
fn observed(app: &mut App, row: FoundRow) {
    worker(app, WorkerEvent::Discovery(DiscoveryEvent::IHave(row)));
}
#[test]
fn passive_observations_and_results_evict_oldest_and_refresh_identity() {
    let mut app = new_app(None);
    app.safety.passive = true;
    worker(
        &mut app,
        WorkerEvent::PassiveListening {
            local: "127.0.0.1:47808".into(),
            foreign_ignored: false,
        },
    );
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "find Supply");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    for n in 0..MAX_OBSERVATIONS {
        observed(&mut app, found(n, "Supply".into()));
    }
    // Refresh 0; the next insert must evict 1, preserving duplicate device IDs.
    observed(&mut app, found(0, "Supply".into()));
    observed(&mut app, found(MAX_OBSERVATIONS, "Supply".into()));
    assert_eq!(app.overheard.0.len(), MAX_OBSERVATIONS);
    assert_eq!(app.who_has.as_ref().unwrap().found.len(), MAX_OBSERVATIONS);
    assert_eq!(app.overheard.0[0].address, "station-00002");
    assert_eq!(
        app.who_has.as_ref().unwrap().found[0].address,
        "station-00002"
    );
    observed(&mut app, found(0, "Renamed".into()));
    assert_eq!(app.overheard.0.last().unwrap().name, "Renamed");
    assert_eq!(
        app.who_has.as_ref().unwrap().found.len(),
        MAX_OBSERVATIONS - 1
    );
    assert_eq!(
        app.names
            .known(12, "station-00000", found(0, String::new()).object),
        Some("Renamed")
    );
    assert_eq!(app.names.len(), crate::tui::app::names::MAX_NAMES);
}
#[test]
fn passive_large_names_hit_the_byte_bound_before_the_row_bound() {
    let mut app = new_app(None);
    worker(
        &mut app,
        WorkerEvent::PassiveListening {
            local: "127.0.0.1:47808".into(),
            foreign_ignored: false,
        },
    );
    for n in 0..200 {
        observed(&mut app, found(n, "x".repeat(60_000)));
    }
    assert!(app.overheard.0.len() < 100);
    let bytes: usize = app
        .overheard
        .0
        .iter()
        .map(|row| row.name.len() + row.address.len())
        .sum();
    assert!(bytes <= MAX_OBSERVATION_BYTES);
    assert_eq!(app.overheard.0.last().unwrap().address, "station-00199");
}
#[test]
fn router_and_network_retention_are_bounded_and_refresh_last_heard() {
    let mut map = RouterMap::default();
    let now = Instant::now();
    for n in 0..MAX_ROUTERS {
        map.record(&format!("router-{n:03}"), &[1], now);
    }
    map.record("router-000", &[1], now + Duration::from_secs(1));
    map.record("new-router", &[2], now + Duration::from_secs(2));
    assert_eq!(map.router_count(), MAX_ROUTERS);
    assert!(map.rows()[0].routers.contains(&"router-000".into()));
    assert!(!map.rows()[0].routers.contains(&"router-001".into()));
    let networks: Vec<_> = (1..=MAX_ROUTER_NETWORKS as u16 + 10).collect();
    map.record("new-router", &networks, now + Duration::from_secs(3));
    let rows: Vec<_> = map
        .rows()
        .into_iter()
        .filter(|row| row.routers.contains(&"new-router".into()))
        .collect();
    assert_eq!(rows.len(), MAX_ROUTER_NETWORKS);
    assert_eq!(rows[0].network, 11);
}

#[test]
fn passive_large_matching_results_keep_the_byte_bound() {
    let name = "x".repeat(60_000);
    let mut search = crate::tui::app::who_has::WhoHasSearch {
        name: name.clone(),
        sent: Instant::now(),
        passive: true,
        found: Vec::new(),
        failed: None,
    };
    for n in 0..200 {
        assert!(search.accept(found(n, name.clone())));
    }
    assert!(search.found.len() < 100);
    let bytes: usize = search
        .found
        .iter()
        .map(|row| row.name.len() + row.address.len())
        .sum();
    assert!(bytes <= MAX_OBSERVATION_BYTES);
    assert_eq!(search.found.last().unwrap().address, "station-00199");
}
