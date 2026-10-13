use super::*;

const FRAME: [u8; 8] = [0x55, 0xFF, 0x00, 0x04, 0x03, 0x00, 0x00, 0x8F];
const GRACE: Duration = Duration::from_millis(40);
const FRAME_ABORT: Duration = Duration::from_millis(100);

fn hunting() -> (EchoFilter, Instant) {
    hunting_with(GRACE)
}

fn hunting_with(grace: Duration) -> (EchoFilter, Instant) {
    let now = Instant::now();
    let mut filter = EchoFilter::new(FRAME_ABORT);
    filter.start(&FRAME, now, grace);
    (filter, now)
}

/// Push `octets` at `now`; return what reaches the core, and whether the
/// echo was dropped.
fn run(filter: &mut EchoFilter, octets: &[u8], now: Instant) -> (Vec<u8>, bool) {
    let mut fed = Vec::new();
    let mut echo = false;
    for &octet in octets {
        let pushed = filter.push(octet, now);
        fed.extend_from_slice(filter.held(pushed.release));
        if pushed.pass {
            fed.push(octet);
        }
        echo |= pushed.echo;
    }
    (fed, echo)
}

#[test]
fn a_whole_echo_is_dropped_and_what_follows_is_fed() {
    let (mut filter, now) = hunting();
    let mut octets = FRAME.to_vec();
    octets.extend_from_slice(&[0x55, 0xFF, 0x00]);
    assert_eq!(
        run(&mut filter, &octets, now),
        (vec![0x55, 0xFF, 0x00], true)
    );
}

#[test]
fn stray_octets_before_the_echo_pass_on_and_the_echo_is_dropped() {
    for stray in [&[0xFF][..], &[0x55], &[0x55, 0xFF], &[0x55, 0x55, 0xFF]] {
        let (mut filter, now) = hunting();
        let mut octets = stray.to_vec();
        octets.extend_from_slice(&FRAME);
        assert_eq!(
            run(&mut filter, &octets, now),
            (stray.to_vec(), true),
            "stray {stray:02x?}"
        );
    }
}

#[test]
fn a_frame_that_only_starts_like_the_echo_passes_on_whole() {
    let (mut filter, now) = hunting();
    let peer = [0x55, 0xFF, 0x00, 0x03, 0x04, 0x00, 0x00, 0x31];
    assert_eq!(run(&mut filter, &peer, now), (peer.to_vec(), false));
}

#[test]
fn a_short_or_corrupted_echo_passes_on() {
    let (mut filter, now) = hunting();
    assert_eq!(
        run(&mut filter, &FRAME[3..], now),
        (FRAME[3..].to_vec(), false)
    );
    let (mut filter, now) = hunting();
    let mut corrupted = FRAME;
    corrupted[4] ^= 0x40;
    assert_eq!(
        run(&mut filter, &corrupted, now),
        (corrupted.to_vec(), false)
    );
}

#[test]
fn the_first_octet_passed_on_is_a_mismatch_once() {
    let (mut filter, now) = hunting();
    assert!(filter.push(0x01, now).first_mismatch);
    assert!(!filter.push(0x02, now).first_mismatch);
}

#[test]
fn a_grace_with_nothing_held_probes_then_releases() {
    let (mut filter, start) = hunting();
    let end = start + GRACE;
    assert_eq!(filter.deadline(), Some(end));
    assert!(filter.timer(end).missing);
    assert_eq!(filter.deadline(), Some(end + GRACE));
    let pushed = filter.push(0x55, end);
    assert!(!pushed.passes() && !pushed.first_mismatch);
    assert_eq!(
        filter.timer(end + GRACE - Duration::from_millis(1)),
        Ended::default()
    );
    assert_eq!(
        filter.timer(end + GRACE),
        Ended {
            release: 1,
            ..Ended::default()
        }
    );
    assert_eq!(filter.deadline(), None);
    assert!(filter.push(0xFF, end + GRACE).pass);
}

#[test]
fn a_grace_with_part_of_the_echo_held_keeps_hunting() {
    let (mut filter, start) = hunting();
    let end = start + GRACE;
    run(&mut filter, &FRAME[..3], start);
    assert!(!filter.timer(end).missing);
    assert_eq!(filter.deadline(), None);
    assert_eq!(
        run(&mut filter, &FRAME[3..], end + Duration::from_millis(5)),
        (vec![], true)
    );
}

#[test]
fn held_octets_go_stale_after_frame_abort() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..2], start);
    let later = start + FRAME_ABORT + Duration::from_millis(1);
    let pushed = filter.push(0x55, later);
    assert_eq!((pushed.stale, pushed.release, pushed.pass), (2, 0, false));
}

#[test]
fn held_octets_are_not_stale_at_exactly_frame_abort() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..2], start);
    assert_eq!(
        filter.push(FRAME[2], start + FRAME_ABORT),
        Pushed::default()
    );
    assert_eq!(
        run(&mut filter, &FRAME[3..], start + FRAME_ABORT),
        (vec![], true)
    );
}

#[test]
fn silence_after_the_grace_ends_the_hunt() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..1], start);
    filter.timer(start + GRACE);
    let later = start + FRAME_ABORT + Duration::from_millis(1);
    let pushed = filter.push(0xFF, later);
    assert_eq!((pushed.stale, pushed.pass), (1, true));
    assert!(filter.push(0x55, later).pass, "the hunt ended");
}

#[test]
fn silence_of_exactly_frame_abort_does_not_end_the_hunt() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..1], start);
    filter.timer(start + GRACE);
    assert_eq!(
        run(&mut filter, &FRAME[1..], start + FRAME_ABORT),
        (vec![], true)
    );
}

/// A grace of 50 ms or more makes the probe outlast T_frame_abort of
/// silence; the probe ends at its own deadline, not at the silence.
#[test]
fn a_late_echo_inside_a_long_probe_is_dropped() {
    let grace = Duration::from_millis(60);
    let (mut filter, start) = hunting_with(grace);
    assert!(filter.timer(start + grace).missing);
    let late = start + Duration::from_millis(105);
    assert_eq!(run(&mut filter, &FRAME, late), (vec![], true));
}

/// What the probe holds at its end passes on, unless it waited longer than
/// T_frame_abort there.
#[test]
fn the_probe_drops_what_waited_longer_than_frame_abort() {
    let grace = Duration::from_millis(150);
    for (read, expected) in [(200, (1, 0)), (199, (0, 1))] {
        let (mut filter, start) = hunting_with(grace);
        filter.timer(start + grace);
        let pushed = filter.push(0x55, start + Duration::from_millis(read));
        assert!(!pushed.passes(), "read at {read} ms");
        let ended = filter.timer(start + grace * 2);
        assert_eq!((ended.release, ended.stale), expected, "read at {read} ms");
    }
}

#[test]
fn a_mismatch_in_the_probe_is_not_counted() {
    let (mut filter, start) = hunting();
    assert!(filter.timer(start + GRACE).missing);
    let pushed = filter.push(0x01, start + GRACE);
    assert!(pushed.pass && !pushed.first_mismatch);
}

/// A grace in which octets passed on counts as a mismatch only, not also
/// as a missing echo; the probe runs either way.
#[test]
fn a_mismatch_is_not_also_missing() {
    let (mut filter, start) = hunting();
    assert!(filter.push(0x01, start).first_mismatch);
    assert_eq!(filter.timer(start + GRACE), Ended::default());
    assert_eq!(filter.deadline(), Some(start + GRACE * 2));
}

#[test]
fn before_a_start_probe_octets_pass_on() {
    let (mut filter, start) = hunting();
    filter.timer(start + GRACE);
    filter.push(0x55, start + GRACE);
    assert_eq!(filter.before_start(start + GRACE).release, 1);
    assert_eq!(filter.deadline(), None);
    assert_eq!(filter.confirmed_start(), 0);
}

/// Octets held in the grace or the residual hunt stay held before a start:
/// a refused start leaves the hunt going, and only a confirmed one drops
/// them, as the echo's continuation.
#[test]
fn held_echo_octets_drop_only_at_a_confirmed_start() {
    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..3], start);
    filter.timer(start + GRACE);
    assert_eq!(filter.before_start(start + GRACE), Ended::default());
    assert_eq!(
        run(&mut filter, &[0x01], start + GRACE),
        (vec![0x55, 0xFF, 0x00, 0x01], false)
    );

    let (mut filter, start) = hunting();
    run(&mut filter, &FRAME[..3], start);
    assert_eq!(filter.before_start(start), Ended::default());
    assert_eq!(filter.confirmed_start(), 3);
    assert_eq!(filter.deadline(), None);
    assert!(filter.push(0x55, start).pass, "the hunt ended");
}
