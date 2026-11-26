//! The rules an I-Am meets in the binding table (#2825): only an answer to
//! the server's own Who-Is binds, and only a check that finds the bound
//! address silent and the claimant answering moves a binding.

use super::*;

const BOUND: &[u8] = &[10, 0, 0, 5, 0xBA, 0xC0];
const CLAIMANT: &[u8] = &[10, 0, 0, 66, 0xBA, 0xC0];
const THIRD: &[u8] = &[10, 0, 0, 77, 0xBA, 0xC0];
const WAIT: Duration = Duration::from_secs(3);

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn now() -> Instant {
    runtime_clock::now()
}

/// Device `instance`'s I-Am from `mac`, on this network.
fn hear(table: &mut DeviceBindingTable, instance: u32, mac: &[u8]) -> Heard {
    let from = IAmFrom {
        mac,
        routed: None,
        local_network: None,
    };
    table.hear_i_am_at(device(instance), from, now(), |_| false, Some(WAIT))
}

/// The MAC Device `instance` resolves to now, if it resolves on this
/// network.
fn bound_at(table: &DeviceBindingTable, instance: u32) -> Option<Vec<u8>> {
    match table.resolve_at(&device(instance), now(), |_| false) {
        DeviceResolution::ResolvedLocal { peer_mac, .. } => Some(peer_mac.to_vec()),
        _ => None,
    }
}

/// A table with Device 9 found at [`BOUND`].
fn found() -> DeviceBindingTable {
    let mut table = DeviceBindingTable::new();
    table.bind_found_at(device(9), BOUND, None, now(), |_| false);
    table
}

/// A table with Device 9 found at [`BOUND`] and a check cued from
/// [`CLAIMANT`], its two Who-Is sent.
fn checking() -> DeviceBindingTable {
    let mut table = found();
    let Heard::Cue(start) = hear(&mut table, 9, CLAIMANT) else {
        panic!("another address cues a check");
    };
    table.check_asked(&start, true);
    table
}

#[tokio::test(start_paused = true)]
async fn only_an_answer_to_the_servers_who_is_binds() {
    let mut table = DeviceBindingTable::new();
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::Ignored);
    assert_eq!(bound_at(&table, 9), None);
    table.ask_for_test(device(9), WAIT);
    assert_eq!(hear(&mut table, 9, BOUND), Heard::Inserted);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));
    // The answer ended the question: the next I-Am from elsewhere is a cue.
    assert!(matches!(hear(&mut table, 9, CLAIMANT), Heard::Cue(_)));
}

#[tokio::test(start_paused = true)]
async fn the_bound_address_refreshes_its_binding_even_once_lapsed() {
    let mut table = found();
    tokio::time::advance(OBSERVED_BINDING_TTL).await;
    assert_eq!(
        table.resolve_at(&device(9), now(), |_| false),
        DeviceResolution::Stale
    );
    assert_eq!(hear(&mut table, 9, BOUND), Heard::Refreshed);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));
}

#[tokio::test(start_paused = true)]
async fn a_configured_binding_is_never_changed_by_traffic() {
    let mut table = DeviceBindingTable::new();
    let binding = DeviceBinding::local(device(9), BOUND).unwrap();
    table.insert_configured(binding, |_| false).unwrap();
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::ConfiguredPreserved);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));
}

/// The claimant answers and the bound address stays silent: the binding
/// moves once the wait is over, not before, its freshness counted from the
/// answer.
#[tokio::test(start_paused = true)]
async fn a_check_moves_the_binding_when_only_the_claimant_answers() {
    let mut table = checking();
    tokio::time::advance(WAIT / 3).await;
    let answered = now();
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::Claimed);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND), "not yet");
    tokio::time::advance(WAIT).await;
    assert_eq!(bound_at(&table, 9).as_deref(), Some(CLAIMANT));
    assert!(matches!(
        table.resolve_at(&device(9), now(), |_| false),
        DeviceResolution::ResolvedLocal {
            freshness: BindingFreshness::ObservedUntil(until),
            ..
        } if until == tokio::time::Instant::from_std(answered + OBSERVED_BINDING_TTL)
    ));
    // The next I-Am writes the move into the table: the claimant is bound.
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::Refreshed);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(CLAIMANT));
}

/// Nothing moves when the bound address answers, the claimant stays
/// silent, a third address claims the device, or the Who-Is did not go
/// out.
#[tokio::test(start_paused = true)]
async fn a_check_keeps_the_binding_otherwise() {
    let mut bound_answers = checking();
    hear(&mut bound_answers, 9, CLAIMANT);
    assert_eq!(hear(&mut bound_answers, 9, BOUND), Heard::Refreshed);
    let mut claimant_silent = checking();
    let mut contested = checking();
    hear(&mut contested, 9, CLAIMANT);
    assert_eq!(hear(&mut contested, 9, THIRD), Heard::Claimed);
    let mut unasked = found();
    let Heard::Cue(start) = hear(&mut unasked, 9, CLAIMANT) else {
        panic!("a cue");
    };
    unasked.check_asked(&start, false);
    hear(&mut unasked, 9, CLAIMANT);
    // Its sender stopped before saying whether the Who-Is went out.
    let mut unreported = found();
    assert!(matches!(hear(&mut unreported, 9, CLAIMANT), Heard::Cue(_)));
    hear(&mut unreported, 9, CLAIMANT);
    tokio::time::advance(WAIT).await;
    for (case, table) in [
        ("bound answers", &mut bound_answers),
        ("claimant silent", &mut claimant_silent),
        ("contested", &mut contested),
        ("not asked", &mut unasked),
        ("not reported", &mut unreported),
    ] {
        assert_eq!(bound_at(table, 9).as_deref(), Some(BOUND), "{case}");
        table.settle_checks(now());
        assert_eq!(bound_at(table, 9).as_deref(), Some(BOUND), "{case}");
    }
}

/// One check per device per hold-off, at most [`MAX_CHECKS`] held at once,
/// and none while the server may not ask.
#[tokio::test(start_paused = true)]
async fn checks_are_paced_capped_and_held_back_by_dcc() {
    let mut table = checking();
    tokio::time::advance(WAIT).await;
    assert_eq!(hear(&mut table, 9, THIRD), Heard::Ignored, "held off");
    tokio::time::advance(CHECK_HOLD_OFF - WAIT).await;
    assert!(matches!(hear(&mut table, 9, THIRD), Heard::Cue(_)));

    let mut table = DeviceBindingTable::new();
    let devices = 100..100 + MAX_CHECKS as u32 + 1;
    for instance in devices.clone() {
        table.bind_found_at(device(instance), BOUND, None, now(), |_| false);
    }
    let cued = devices
        .filter(|instance| matches!(hear(&mut table, *instance, CLAIMANT), Heard::Cue(_)))
        .count();
    assert_eq!(cued, MAX_CHECKS);

    let mut table = found();
    let from = IAmFrom {
        mac: CLAIMANT,
        routed: None,
        local_network: None,
    };
    let heard = table.hear_i_am_at(device(9), from, now(), |_| false, None);
    assert_eq!(heard, Heard::Ignored);
}

/// A lapsed binding asked for again: the old address answering takes it
/// back at once; another takes it only once the wait is over with the old
/// one silent; two others decide nothing and keep the lapsed binding; none
/// drops it.
#[tokio::test(start_paused = true)]
async fn a_lapsed_binding_moves_only_when_its_old_address_stays_silent() {
    let [mut old_answers, mut moved, mut contested, mut silent] = [(); 4].map(|()| found());
    tokio::time::advance(OBSERVED_BINDING_TTL).await;
    for table in [&mut old_answers, &mut moved, &mut contested, &mut silent] {
        table.ask_for_test(device(9), WAIT);
    }

    hear(&mut old_answers, 9, CLAIMANT);
    assert_eq!(hear(&mut old_answers, 9, BOUND), Heard::Refreshed);
    assert_eq!(bound_at(&old_answers, 9).as_deref(), Some(BOUND));

    assert_eq!(hear(&mut moved, 9, CLAIMANT), Heard::Claimed);
    moved.after_probe(&device(9), now());
    assert_eq!(bound_at(&moved, 9), None, "not before the wait is over");

    hear(&mut contested, 9, CLAIMANT);
    hear(&mut contested, 9, THIRD);

    tokio::time::advance(WAIT).await;
    moved.after_probe(&device(9), now());
    assert_eq!(bound_at(&moved, 9).as_deref(), Some(CLAIMANT));
    contested.after_probe(&device(9), now());
    assert_eq!(
        contested.resolve_at(&device(9), now(), |_| false),
        DeviceResolution::Stale
    );
    silent.after_probe(&device(9), now());
    assert_eq!(
        silent.resolve_at(&device(9), now(), |_| false),
        DeviceResolution::Unknown
    );
}

/// A device bound through this network's own number is the station it
/// names here: its I-Am from that MAC, with no SNET, refreshes the binding.
#[tokio::test(start_paused = true)]
async fn a_binding_through_this_networks_number_takes_its_stations_direct_i_am() {
    let mut table = DeviceBindingTable::new();
    let relayed = NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(&[0x09]),
    };
    table.bind_found_at(device(9), BOUND, Some(&relayed), now(), |_| false);
    let from = IAmFrom {
        mac: &[0x09],
        routed: None,
        local_network: Some(5),
    };
    let heard = table.hear_i_am_at(device(9), from, now(), |_| false, Some(WAIT));
    assert_eq!(heard, Heard::Refreshed);
}
