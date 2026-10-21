//! Learning claims under standard and hardened convergence. Ported from
//! `bacnet-network`'s `router_table_corroboration_tests.rs` and the
//! convergence cases of `router/rb06_convergence_tests.rs`.

mod support;

use bacnet_router_core::npdu::Mac;
use bacnet_router_core::npdu::RejectMessageReason;
use bacnet_router_core::table::{Convergence, Learned, Reachability, RejectEffect};
use bacnet_router_core::table::{LearnRefusal, Timing};
use bacnet_router_core::{Instant, PortId};
use support::{
    claim_counts, expire_pending, hardened, learned_hardened as learned_table, mac, port, purge,
    secs, Hosted, BASE, CORROBORATION_WINDOW, HOLD_DOWN, NANO, TIMING,
};

fn claim(table: &mut Hosted, port: u8, now: Instant) -> bool {
    table
        .apply_claim(3000, PortId(port), mac(&[2]), now, &TIMING)
        .applied()
}

#[test]
fn corroboration_inclusive_deadline_and_expiry_restart() {
    for elapsed in [
        CORROBORATION_WINDOW - NANO,
        CORROBORATION_WINDOW,
        CORROBORATION_WINDOW + NANO,
    ] {
        let mut table = learned_table();
        let now = BASE;
        let seen = table.lookup(3000).unwrap().last_seen();
        assert_eq!(
            table.apply_claim(3000, port(1), mac(&[2]), now, &TIMING),
            Learned::Held
        );
        assert_eq!(table.lookup(3000).unwrap().port(), port(0));
        assert_eq!(table.lookup(3000).unwrap().last_seen(), seen);
        assert_eq!(table.lookup(3000).unwrap().next_hop().as_slice(), &[1]);
        assert_eq!(
            table.claim_counters(),
            claim_counts(|c| c.pending_started = 1)
        );

        let applies = elapsed <= CORROBORATION_WINDOW;
        assert_eq!(claim(&mut table, 1, now + elapsed), applies);
        assert_eq!(
            table.lookup(3000).unwrap().port(),
            PortId(u8::from(applies))
        );
        assert_eq!(table.pending_count() == 0, applies);
        assert_eq!(
            table.claim_counters(),
            claim_counts(|c| {
                c.pending_started = if applies { 1 } else { 2 };
                c.corroborated_applied = u64::from(applies);
                c.pending_expired = u64::from(!applies);
            })
        );
        if !applies {
            assert_eq!(table.pending(3000).unwrap().first_seen, now + elapsed);
            assert_eq!(table.lookup(3000).unwrap().last_seen(), seen);
            assert!(claim(&mut table, 1, now + elapsed + CORROBORATION_WINDOW));
            assert_eq!(table.pending_count(), 0);
            assert_eq!(table.claim_counters().corroborated_applied, 1);
            assert_eq!(table.claim_counters().pending_expired, 1);
        }
    }
}

#[test]
fn spaced_single_claims_never_accumulate_or_refresh_old_route() {
    let mut table = learned_table();
    let now = BASE;
    let seen = table.lookup(3000).unwrap().last_seen();
    for index in 0..10u32 {
        let at = now + (CORROBORATION_WINDOW + NANO).saturating_mul(index);
        assert!(!claim(&mut table, 1, at));
        assert_eq!(table.lookup(3000).unwrap().port(), port(0));
        assert_eq!(table.lookup(3000).unwrap().last_seen(), seen);
        assert_eq!(table.pending_count(), 1);
        assert_eq!(
            table.claim_counters(),
            claim_counts(|c| {
                c.pending_started = u64::from(index + 1);
                c.pending_expired = u64::from(index);
            })
        );
    }
}

#[test]
fn alternating_challengers_replace_slot_and_reset_first_seen() {
    let mut table = learned_table();
    let now = BASE;
    for second in 0..100u64 {
        let challenger = 1 + (second % 2) as u8;
        let time = now + secs(second);
        assert!(!claim(&mut table, challenger, time));
        assert_eq!(table.lookup(3000).unwrap().port(), port(0));
        assert_eq!(table.pending_count(), 1);
        assert_eq!(table.pending(3000).unwrap().port, port(challenger));
        assert_eq!(table.pending(3000).unwrap().first_seen, time);
    }
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| c.pending_started = 100)
    );
    // Legitimate convergence resumes when a challenger can repeat uninterrupted.
    assert!(claim(&mut table, 2, now + secs(100)));
    assert_eq!(table.lookup(3000).unwrap().port(), port(2));
    assert_eq!(table.claim_counters().corroborated_applied, 1);
}

#[test]
fn absent_current_port_direct_and_reserved_claims_remain_immediate() {
    let mut table = hardened();
    let now = BASE;
    assert!(claim(&mut table, 0, now));
    assert_eq!(table.pending_count(), 0);
    assert!(!claim(&mut table, 1, now));
    table.mark_busy(3000, now + HOLD_DOWN);
    assert!(claim(&mut table, 0, now));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port(), port(0));
    assert_eq!(entry.reachability(), Reachability::Reachable);
    assert_eq!(entry.flap_count(), 0);
    assert!(entry.last_port_change().is_none());
    assert!(entry.busy_until().is_none());
    assert_eq!(table.pending_count(), 0);
    assert!(!claim(&mut table, 1, now)); // The refresh discarded the earlier vote.
    table.add_direct(3000, port(0));
    for (net, refusal) in [
        (0, LearnRefusal::Reserved),
        (0xffff, LearnRefusal::Reserved),
        (3000, LearnRefusal::Direct),
    ] {
        for _ in 0..2 {
            assert_eq!(
                table.apply_claim(net, port(1), Mac::EMPTY, now, &TIMING),
                Learned::Refused(refusal)
            );
        }
    }
    assert_eq!(table.len(), 1);
    assert!(table.lookup(3000).unwrap().is_direct());
    assert_eq!(table.pending_count(), 0);
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| c.pending_started = 2)
    );
}

#[test]
fn pending_claim_preserves_busy_and_reject_records_until_apply() {
    let mut table = learned_table();
    let now = BASE;
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        now,
        &TIMING,
    );
    assert!(!claim(&mut table, 1, now));
    assert_eq!(
        table.lookup(3000).unwrap().busy_until(),
        Some(now + HOLD_DOWN)
    );
    // The busy mark's hold-down holds off a reason-1 change.
    assert_eq!(
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            now,
            &TIMING
        ),
        RejectEffect::HeldDown
    );
    assert_eq!(table.claim_counters().reject_dampened_hold_down, 1);
    assert!(claim(&mut table, 1, now));
    // The move cleared the hold-down: the same reject now applies.
    let mut probe = table.clone();
    assert_eq!(
        probe.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            now,
            &TIMING
        ),
        RejectEffect::Applied
    );
    assert_eq!(
        table.lookup(3000).unwrap().reachability(),
        Reachability::Reachable
    );
    assert_eq!(table.claim_counters().corroborated_applied, 1);
}

#[test]
fn cleanup_never_leaves_pending_without_a_learned_route() {
    for cleanup in ["remove", "aging", "direct", "manual", "manual_flap"] {
        let mut table = learned_table();
        let now = BASE;
        // Confirm the route 120 s back before the claim, so "aging" purges
        // it.
        assert!(table.add_learned(3000, port(0), mac(&[1]), now - secs(120)));
        assert!(!claim(&mut table, 1, now));
        match cleanup {
            "remove" => {
                table.remove(3000);
            }
            "aging" => assert_eq!(purge(&mut table, now, secs(60)), [3000]),
            "direct" => {
                table.add_direct(3000, port(1));
            }
            "manual" => {
                table.add_learned(3000, port(2), Mac::EMPTY, now);
            }
            "manual_flap" => {
                assert!(table.learn(3000, port(2), Mac::EMPTY, now).applied());
            }
            _ => unreachable!(),
        }
        assert_eq!(table.pending_count(), 0, "{cleanup}");
        assert_eq!(table.claim_counters().pending_expired, 0, "{cleanup}");
        table.add_learned(4000, port(0), Mac::EMPTY, now);
        assert!(table
            .apply_claim(4000, port(0), Mac::EMPTY, now, &TIMING)
            .applied());
        assert!(!table
            .apply_claim(4000, port(1), Mac::EMPTY, now, &TIMING)
            .applied());
        assert_eq!(table.pending_count(), 1);
    }
}

#[test]
fn pending_store_is_bounded_and_networks_are_independent() {
    let mut table = hardened();
    let now = BASE;
    for net in 1..=256u16 {
        table.add_learned(net, port(0), Mac::EMPTY, now);
        for challenger in 1..=10 {
            assert!(!table
                .apply_claim(net, port(challenger), Mac::EMPTY, now, &TIMING)
                .applied());
            assert_eq!(table.pending_count(), usize::from(net));
            let learned = table
                .routes()
                .filter(|(_, route)| !route.is_direct())
                .count();
            assert!(table.pending_count() <= learned);
        }
    }
    for net in 1..=256u16 {
        assert!(table
            .apply_claim(net, port(10), Mac::EMPTY, now, &TIMING)
            .applied());
        assert_eq!(table.pending_count(), 256 - usize::from(net));
    }
    assert_eq!(
        table.claim_counters(),
        claim_counts(|c| {
            c.pending_started = 2560;
            c.corroborated_applied = 256;
        })
    );
}

#[test]
fn flap_warnings_only_count_applied_moves_and_same_port_refresh_resets_history() {
    let mut table = learned_table();
    let now = BASE;
    for challenger in 1..=3u8 {
        assert!(!claim(&mut table, challenger, now));
        assert_eq!(table.claim_counters().flap_warned, 0);
        assert!(claim(&mut table, challenger, now));
        assert_eq!(table.lookup(3000).unwrap().flap_count(), challenger);
    }
    assert_eq!(table.claim_counters().flap_warned, 1);
    assert!(claim(&mut table, 3, now));
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 0);
    assert!(!claim(&mut table, 4, now));
    assert!(claim(&mut table, 4, now));
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 1);
    assert_eq!(table.claim_counters().flap_warned, 1);
}

// --- Convergence modes (rb06_convergence_tests.rs) ---

#[test]
fn standard_converges_on_single_claim_under_sustained_traffic() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    assert_eq!(table.convergence(), Convergence::Standard);
    for i in 1..=50 {
        table.touch(3000, BASE + secs(i));
    }
    assert_eq!(
        table.apply_claim(3000, port(1), mac(&[2]), BASE + secs(51), &TIMING),
        Learned::Moved {
            from: port(0),
            flaps: 1
        }
    );
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port(), port(1));
    assert_eq!(entry.next_hop().as_slice(), &[2]);
    assert_eq!(entry.last_seen(), Some(BASE + secs(51)));
    assert!(entry.last_used().is_none());
    assert_eq!(table.pending_count(), 0);
    assert_eq!(table.claim_counters().pending_started, 0);
}

#[test]
fn hardened_holds_then_reaps_then_recovers() {
    let mut table = hardened();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    assert!(!table
        .apply_claim(3000, port(1), mac(&[2]), BASE, &TIMING)
        .applied());
    assert_eq!(table.lookup(3000).unwrap().port(), port(0));
    assert_eq!(table.pending_count(), 1);
    // The sweep clears the slot so a single-shot advertiser does not linger:
    // not at the window's end, which still counts, but past it.
    assert!(expire_pending(&mut table, BASE + CORROBORATION_WINDOW).is_empty());
    assert_eq!(expire_pending(&mut table, BASE + secs(61)), [3000]);
    assert_eq!(table.pending_count(), 0);
    assert_eq!(table.claim_counters().pending_expired, 1);
    // A fresh challenger pends again, and an immediate repeat corroborates.
    assert!(!table
        .apply_claim(3000, port(1), mac(&[2]), BASE + secs(62), &TIMING)
        .applied());
    assert!(table
        .apply_claim(3000, port(1), mac(&[2]), BASE + secs(63), &TIMING)
        .applied());
    assert_eq!(table.lookup(3000).unwrap().port(), port(1));
    assert_eq!(table.claim_counters().corroborated_applied, 1);
}

#[test]
fn hardened_alternation_never_converges_robustness_only() {
    // Cyclic alternation is a bounded-robustness check, never a supported
    // topology claim: the slot resets each time and the old route forwards.
    let mut table = hardened();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    for i in 0..10u64 {
        assert!(!table
            .apply_claim(
                3000,
                port(1 + (i % 2) as u8),
                mac(&[2]),
                BASE + secs(i),
                &TIMING
            )
            .applied());
        assert_eq!(table.lookup(3000).unwrap().port(), port(0));
    }
    assert_eq!(table.pending_count(), 1);
    assert_eq!(table.claim_counters().pending_started, 10);
}

#[test]
fn the_corroboration_window_follows_the_timing_and_the_mode_can_change() {
    let mut table = learned_table();
    let timing = Timing {
        corroboration: secs(5),
        ..Timing::DEFAULT
    };
    let short = |table: &mut Hosted, port: u8, now: Instant| {
        table
            .apply_claim(3000, PortId(port), mac(&[2]), now, &timing)
            .applied()
    };
    assert!(!short(&mut table, 1, BASE));
    assert!(!short(&mut table, 1, BASE + secs(5) + NANO));
    assert!(short(&mut table, 1, BASE + secs(10)));
    // The sweep reads the same window.
    assert!(!short(&mut table, 2, BASE + secs(11)));
    let mut expired = Vec::new();
    table.expire_pending(BASE + secs(16), &timing, |net| expired.push(net));
    assert!(expired.is_empty());
    table.expire_pending(BASE + secs(16) + NANO, &timing, |net| expired.push(net));
    assert_eq!(expired, [3000]);

    // Back to standard: the next claim moves the route at once, and drops
    // whatever was pending without counting it.
    assert!(!claim(&mut table, 2, BASE + secs(11)));
    table.set_convergence(Convergence::Standard);
    let expired = table.claim_counters().pending_expired;
    assert!(claim(&mut table, 3, BASE + secs(12)));
    assert_eq!(table.pending_count(), 0);
    assert_eq!(table.claim_counters().pending_expired, expired);
    assert_eq!(table.lookup(3000).unwrap().port(), port(3));
}
