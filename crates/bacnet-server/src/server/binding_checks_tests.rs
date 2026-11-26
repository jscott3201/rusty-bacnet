//! The rules an I-Am meets in the binding table (#2825): only an answer to
//! the server's own Who-Is binds, and only two checks a hold-off apart that
//! both find the bound address failing, while the same claimant answers and
//! the bound address answers nothing else, move a binding.

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

/// Device 9's check cued by `claimant`'s I-Am.
fn cue(table: &mut DeviceBindingTable, claimant: &[u8]) -> CheckStart {
    let Heard::Cue(start) = hear(table, 9, claimant) else {
        panic!("another address cues a check");
    };
    start
}

/// Begin `start`'s next check now: whether it may.
fn begins(table: &mut DeviceBindingTable, start: &CheckStart) -> bool {
    table.check_begins(start, now(), |_| false).is_some()
}

/// End `start`'s check now, the bound address having said `identity`.
fn ends(table: &mut DeviceBindingTable, start: &CheckStart, identity: Identity) -> Step {
    let checked = Checked {
        identity,
        heard: false,
    };
    table.check_ended(start, checked, now())
}

/// One check of Device 9 in which the claimant answers and the bound
/// address says `identity`.
fn check(table: &mut DeviceBindingTable, start: &CheckStart, identity: Identity) -> Step {
    assert!(begins(table, start));
    assert_eq!(hear(table, 9, CLAIMANT), Heard::Claimed);
    ends(table, start, identity)
}

/// A table with Device 9 found at [`BOUND`] and a first check cued by
/// [`CLAIMANT`] that struck, with the start that runs it.
fn struck() -> (DeviceBindingTable, CheckStart) {
    let mut table = found();
    let start = cue(&mut table, CLAIMANT);
    assert_eq!(check(&mut table, &start, Identity::Silent), Step::Again);
    (table, start)
}

#[tokio::test(start_paused = true)]
async fn only_an_answer_to_the_servers_who_is_binds() {
    let mut table = DeviceBindingTable::new();
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::Ignored);
    assert_eq!(bound_at(&table, 9), None);
    table.ask_for_test(device(9), WAIT);
    assert_eq!(hear(&mut table, 9, BOUND), Heard::Claimed);
    table.decide_for_test(device(9));
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));
    // The device is bound: the next I-Am from elsewhere is a cue.
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

/// Two checks a hold-off apart that both strike move the binding to the
/// claimant, its freshness counted from the claimant's last answer; one
/// strike moves nothing.
#[tokio::test(start_paused = true)]
async fn two_strikes_a_hold_off_apart_move_the_binding() {
    let (mut table, start) = struck();
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND), "one strike");
    assert_eq!(hear(&mut table, 9, CLAIMANT), Heard::Ignored, "between");
    tokio::time::advance(CHECK_HOLD_OFF).await;
    assert!(begins(&mut table, &start));
    let answered = now();
    hear(&mut table, 9, CLAIMANT);
    tokio::time::advance(WAIT).await;
    let moved = ends(&mut table, &start, Identity::Is(10));
    let [from, to] = [BOUND, CLAIMANT].map(|mac| DeviceBindingTarget::Local {
        peer_mac: MacAddr::from_slice(mac),
    });
    assert_eq!(moved, Step::Moved(from, to));
    assert!(matches!(
        table.resolve_at(&device(9), now(), |_| false),
        DeviceResolution::ResolvedLocal {
            peer_mac,
            freshness: BindingFreshness::ObservedUntil(until),
        } if peer_mac.as_slice() == CLAIMANT
            && until == tokio::time::Instant::from_std(answered + OBSERVED_BINDING_TTL)
    ));
}

/// A check fails when the bound address names another instance, has no
/// Device object of the bound one, or stays silent through its retries; it
/// answers for the device, or unclearly (a Reject, an Abort, another
/// Error), or the read didn't all go out, and it doesn't.
#[tokio::test(start_paused = true)]
async fn what_fails_a_check() {
    for (identity, fails) in [
        (Identity::Is(10), true),
        (Identity::NotThere, true),
        (Identity::Silent, true),
        (Identity::Is(9), false),
        (Identity::Unclear, false),
        (Identity::NotAsked, false),
    ] {
        let mut table = found();
        let start = cue(&mut table, CLAIMANT);
        let step = check(&mut table, &start, identity);
        assert_eq!(step == Step::Again, fails, "{identity:?}");
    }
}

/// A failed check strikes only when the claimant answered its Who-Is, no
/// third address claimed the device, and the bound address answered no
/// other request of the server's.
#[tokio::test(start_paused = true)]
async fn a_failed_check_strikes_only_with_the_claimant_alone_answering() {
    let mut silent = found();
    let start = cue(&mut silent, CLAIMANT);
    assert!(begins(&mut silent, &start));
    assert_eq!(ends(&mut silent, &start, Identity::Silent), Step::Done);

    let mut contested = found();
    let start = cue(&mut contested, CLAIMANT);
    assert!(begins(&mut contested, &start));
    hear(&mut contested, 9, CLAIMANT);
    assert_eq!(hear(&mut contested, 9, THIRD), Heard::Claimed);
    assert_eq!(ends(&mut contested, &start, Identity::Silent), Step::Done);

    let mut heard = found();
    let start = cue(&mut heard, CLAIMANT);
    assert!(begins(&mut heard, &start));
    hear(&mut heard, 9, CLAIMANT);
    let checked = Checked {
        identity: Identity::Silent,
        heard: true,
    };
    assert_eq!(heard.check_ended(&start, checked, now()), Step::Done);

    // An I-Am of the claimant's before its Who-Is went out answers nothing.
    let mut early = found();
    let start = cue(&mut early, CLAIMANT);
    hear(&mut early, 9, CLAIMANT);
    assert!(begins(&mut early, &start));
    assert_eq!(ends(&mut early, &start, Identity::Silent), Step::Done);
}

/// After one strike, anything but a second strike ends the checks with the
/// binding where it was: the bound address answering, another claimant, a
/// check that didn't go out, the bound address heard from, or the binding
/// lapsing. The next cue starts again from no strike.
#[tokio::test(start_paused = true)]
async fn a_second_check_that_does_not_strike_resets() {
    for (case, identity) in [
        ("the old address answers", Identity::Is(9)),
        ("it answers unclearly", Identity::Unclear),
        ("not asked", Identity::NotAsked),
    ] {
        let (mut table, start) = struck();
        tokio::time::advance(CHECK_HOLD_OFF).await;
        assert_eq!(check(&mut table, &start, identity), Step::Done, "{case}");
        assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND), "{case}");
        tokio::time::advance(CHECK_HOLD_OFF).await;
        let again = cue(&mut table, CLAIMANT);
        assert_eq!(check(&mut table, &again, Identity::Silent), Step::Again);
    }

    // Another station claims the device during the second check, the
    // first claimant silent.
    let (mut table, start) = struck();
    tokio::time::advance(CHECK_HOLD_OFF).await;
    assert!(begins(&mut table, &start));
    hear(&mut table, 9, THIRD);
    assert_eq!(ends(&mut table, &start, Identity::Silent), Step::Done);

    // The old address answered a request of the server's between them.
    let (mut table, start) = struck();
    tokio::time::advance(CHECK_HOLD_OFF).await;
    assert!(begins(&mut table, &start));
    hear(&mut table, 9, CLAIMANT);
    let checked = Checked {
        identity: Identity::Silent,
        heard: true,
    };
    assert_eq!(table.check_ended(&start, checked, now()), Step::Done);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));

    // The binding lapsed before the second check: it doesn't begin.
    let (mut table, start) = struck();
    tokio::time::advance(OBSERVED_BINDING_TTL).await;
    assert!(!begins(&mut table, &start));
}

/// One check per device per hold-off from a cue, at most [`MAX_CHECKS`]
/// held at once, a device between its two checks never given up for a new
/// cue, and none while the server may not ask.
#[tokio::test(start_paused = true)]
async fn checks_are_paced_capped_and_held_back_by_dcc() {
    let mut table = found();
    let start = cue(&mut table, CLAIMANT);
    assert_eq!(check(&mut table, &start, Identity::Is(9)), Step::Done);
    tokio::time::advance(WAIT).await;
    assert_eq!(hear(&mut table, 9, THIRD), Heard::Ignored, "held off");
    tokio::time::advance(CHECK_HOLD_OFF - WAIT).await;
    assert!(matches!(hear(&mut table, 9, THIRD), Heard::Cue(_)));

    let mut table = DeviceBindingTable::new();
    let devices = 100..100 + MAX_CHECKS as u32 + 1;
    for instance in devices.clone() {
        table.bind_found_at(device(instance), BOUND, None, now(), |_| false);
    }
    let starts: Vec<_> = devices
        .clone()
        .filter_map(|instance| match hear(&mut table, instance, CLAIMANT) {
            Heard::Cue(start) => Some(start),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), MAX_CHECKS);
    // Each strikes once and waits for its second check, a hold-off on.
    for start in &starts {
        assert!(table.check_begins(start, now(), |_| false).is_some());
        let instance = start.device.instance_number();
        hear(&mut table, instance, CLAIMANT);
        assert_eq!(ends(&mut table, start, Identity::Silent), Step::Again);
    }
    tokio::time::advance(CHECK_HOLD_OFF * 2).await;
    let last = devices.end - 1;
    assert_eq!(hear(&mut table, last, CLAIMANT), Heard::Ignored, "full");

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

/// A late answer to the server's own Who-Is, the only one it drew, still
/// binds while the probe holds the device back from another Who-Is.
#[tokio::test(start_paused = true)]
async fn a_sole_late_answer_binds_within_the_hold_off() {
    let mut table = DeviceBindingTable::new();
    table.ask_for_test(device(9), WAIT);
    tokio::time::advance(WAIT + Duration::from_secs(10)).await;
    assert_eq!(hear(&mut table, 9, BOUND), Heard::Inserted);
    assert_eq!(bound_at(&table, 9).as_deref(), Some(BOUND));
}

/// First contact is decided when the server's Who-Is's wait ends (decision,
/// maintainer, 2026-11-26): one station answered, the real device or a
/// forger with the device absent, and it is bound; two or more, and none
/// is, counted once; a late answer from another station contests too.
#[tokio::test(start_paused = true)]
async fn first_contact_binds_only_a_sole_answer_once_the_wait_ends() {
    let mut both = DeviceBindingTable::new();
    both.ask_for_test(device(9), WAIT);
    assert_eq!(hear(&mut both, 9, BOUND), Heard::Claimed);
    assert_eq!(hear(&mut both, 9, CLAIMANT), Heard::Claimed);
    assert_eq!(bound_at(&both, 9), None, "nothing before the wait ends");
    let mut alone = DeviceBindingTable::new();
    alone.ask_for_test(device(9), WAIT);
    hear(&mut alone, 9, CLAIMANT);
    let mut late = DeviceBindingTable::new();
    late.ask_for_test(device(9), WAIT);
    hear(&mut late, 9, BOUND);
    tokio::time::advance(WAIT).await;
    assert_eq!(hear(&mut late, 9, CLAIMANT), Heard::Claimed);
    for _ in 0..2 {
        for table in [&mut both, &mut alone, &mut late] {
            table.after_probe(&device(9), now());
        }
    }
    assert_eq!(bound_at(&both, 9), None);
    assert_eq!(both.conflicts(), 1, "counted once");
    assert_eq!(bound_at(&alone, 9).as_deref(), Some(CLAIMANT));
    assert_eq!(alone.conflicts(), 0);
    assert_eq!(bound_at(&late, 9), None);
}
