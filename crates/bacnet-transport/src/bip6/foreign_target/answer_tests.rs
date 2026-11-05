//! Matching the BBMD's results to a B/IPv6 foreign device's registrations
//! (#2388), on the target alone: no socket, and time paused for the ages.

use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bacnet_bip_codec::ipv6::Bvlc6ResultCode;
use tokio::time::{Instant, advance};

use super::*;

const SUCCESS: Bvlc6ResultCode = Bvlc6ResultCode::SUCCESSFUL_COMPLETION;
const NAK: Bvlc6ResultCode = Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK;

fn bbmd(last: u16) -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, last),
        47808,
        0,
        0,
    )
}

/// The registration with `bbmd`, `ttl` 60.
fn config(bbmd: SocketAddrV6) -> Bip6ForeignDeviceConfig {
    Bip6ForeignDeviceConfig {
        bbmd: bbmd.into(),
        ttl: 60,
    }
}

/// A renewal pass of `target`, its registration sent.
fn renew(target: &ForeignTarget) {
    target.pass(Instant::now(), Cause::Renewal);
    target.registration_sent();
}

/// The answer, unanswered streak and refusals `target` reports.
fn answer(target: &ForeignTarget) -> (Option<Bip6RegistrationAnswer>, u64, u64) {
    let status = target.status();
    (
        status.last_answer,
        status.unanswered,
        status.registrations_refused,
    )
}

/// A success marks the registration held, and the status says how long
/// ago; the answer stands until the next one.
#[tokio::test(start_paused = true)]
async fn a_success_marks_the_registration_held() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    assert_eq!(answer(&target), (None, 0, 0));
    renew(&target);
    // Sent, not answered yet.
    assert_eq!(answer(&target), (None, 0, 0));
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
    assert_eq!(target.status().last_answer_age, Some(Duration::ZERO));
    advance(Duration::from_secs(12)).await;
    assert_eq!(
        target.status().last_answer_age,
        Some(Duration::from_secs(12))
    );
}

/// A NAK marks it refused and counts it; the next registration's success
/// marks it held again, and the total stays.
#[tokio::test(start_paused = true)]
async fn a_nak_marks_the_registration_refused_and_counts_it() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    renew(&target);
    target.result(a, NAK);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Refused), 0, 1)
    );
    // The renewal goes as usual, and the refusal stands until answered.
    renew(&target);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Refused), 0, 1)
    );
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 1));
}

/// Only a success or the registration's NAK, from the BBMD the outstanding
/// registration went to, answers it, and only once.
#[tokio::test(start_paused = true)]
async fn only_the_outstanding_registration_is_answered() {
    let (a, b) = (bbmd(1), bbmd(2));
    let target = ForeignTarget::at(a);
    // Nothing outstanding yet.
    target.result(a, SUCCESS);
    target.result(a, NAK);
    assert_eq!(answer(&target), (None, 0, 0));
    renew(&target);
    // Another peer, or another request's NAK.
    target.result(b, SUCCESS);
    target.result(b, NAK);
    target.result(a, Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK);
    target.result(a, Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK);
    target.result(a, Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK);
    assert_eq!(answer(&target), (None, 0, 0));
    // The same address with a zone index is the same BBMD.
    target.result(SocketAddrV6::new(*a.ip(), a.port(), 0, 7), SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
    // A second answer finds nothing outstanding.
    target.result(a, NAK);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}

/// A registration still unanswered when the next goes is unanswered, and
/// the streak counts each one until an answer comes.
#[tokio::test(start_paused = true)]
async fn no_answer_by_the_next_registration_is_unanswered() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    renew(&target);
    advance(Duration::from_secs(30)).await;
    renew(&target);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Unanswered), 1, 0)
    );
    assert_eq!(target.status().last_answer_age, Some(Duration::ZERO));
    renew(&target);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Unanswered), 2, 0)
    );
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}

/// After a move, the old BBMD's success answers the release in flight
/// (#2338), not the registration with the new one.
#[tokio::test(start_paused = true)]
async fn a_success_for_a_release_is_not_a_registrations() {
    let (a, b) = (bbmd(1), bbmd(2));
    let target = ForeignTarget::at(a);
    target.set_release_previous(true);
    target.lock().own = Some(bbmd(9));
    renew(&target);
    target.result(a, SUCCESS);
    // A move to B: the pass registers there and releases A.
    target.lock().take(config(b));
    let (_, release) = target.pass(Instant::now(), Cause::Change);
    assert_eq!(release.map(|(left, _)| left), Some(a));
    target.registration_sent();
    target.result(a, SUCCESS);
    assert_eq!(target.release_counters().released, 1);
    assert_eq!(answer(&target), (None, 0, 0));
    target.result(b, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}

/// The release takes a success from its BBMD even while a registration
/// there is outstanding; the next success is the registration's.
#[tokio::test(start_paused = true)]
async fn a_release_and_a_registration_to_one_bbmd_take_one_success_each() {
    let (a, b) = (bbmd(1), bbmd(2));
    let target = ForeignTarget::at(a);
    target.set_release_previous(true);
    target.lock().own = Some(bbmd(9));
    // A to B releases A; with the release off, back to A releases nothing,
    // so A's release still waits when the registration to A goes.
    target.lock().take(config(b));
    target.pass(Instant::now(), Cause::Change);
    target.set_release_previous(false);
    target.lock().take(config(a));
    target.pass(Instant::now(), Cause::Change);
    target.registration_sent();
    target.result(a, SUCCESS);
    assert_eq!(target.release_counters().released, 1);
    assert_eq!(answer(&target), (None, 0, 0));
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}

/// A move to another BBMD clears what the old one said, and its late
/// answers count for nothing; a new time-to-live alone keeps it.
#[tokio::test(start_paused = true)]
async fn a_bbmd_change_clears_the_old_answer() {
    let (a, b) = (bbmd(1), bbmd(2));
    let target = ForeignTarget::at(a);
    renew(&target);
    target.result(a, SUCCESS);
    let mut longer = config(a);
    longer.ttl = 120;
    target.lock().take(longer);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));

    renew(&target);
    renew(&target);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Unanswered), 1, 0)
    );
    target.lock().take(config(b));
    assert_eq!(answer(&target), (None, 0, 0));
    // A's answer to its outstanding registration comes too late.
    target.result(a, NAK);
    assert_eq!(answer(&target), (None, 0, 0));
    renew(&target);
    target.result(b, NAK);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Refused), 0, 1)
    );
}

/// A device that waits for its BBMD's name has no answer, and a stop
/// clears it; the refusal total survives both.
#[tokio::test(start_paused = true)]
async fn waiting_and_stopping_clear_the_answer() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    renew(&target);
    target.result(a, NAK);
    target.detach();
    assert_eq!(answer(&target), (None, 0, 1));

    renew(&target);
    target.result(a, NAK);
    let named = Bip6ForeignDeviceConfig {
        bbmd: "bbmd.example:47808".parse().unwrap(),
        ttl: 60,
    };
    target.lock().take(named);
    assert_eq!(answer(&target), (None, 0, 2));
}

/// A start's registration to a BBMD the device has since left starts
/// nothing outstanding; one to the address the start's lookup found does,
/// though the device still waits for the name until the start takes that
/// address.
#[tokio::test(start_paused = true)]
async fn only_a_registration_to_the_bbmd_in_use_waits_for_an_answer() {
    let (a, b) = (bbmd(1), bbmd(2));
    let target = ForeignTarget::at(b);
    target.registration_going(a);
    target.registration_sent();
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (None, 0, 0));
    assert_eq!(target.status().registrations_sent, 1);

    let named = Bip6ForeignDeviceConfig {
        bbmd: "bbmd.example:47808".parse().unwrap(),
        ttl: 60,
    };
    let target = ForeignTarget::new(named, Arc::new(crate::bbmd_lookup::SystemResolver));
    target.registration_going(a);
    target.registration_sent();
    target.take_found(a);
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}

/// A registration is outstanding from its pass, before the send returns,
/// so an answer that beats the bookkeeping still counts.
#[tokio::test(start_paused = true)]
async fn a_registration_awaits_its_answer_before_it_is_sent() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    target.pass(Instant::now(), Cause::Renewal);
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
    target.registration_sent();
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
    assert_eq!(target.status().registrations_sent, 1);
}

/// A registration that couldn't be sent awaits no answer and goes
/// unanswered at no renewal.
#[tokio::test(start_paused = true)]
async fn a_failed_send_is_not_an_unanswered_registration() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    target.pass(Instant::now(), Cause::Renewal);
    target.registration_not_sent(a);
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (None, 0, 0));
    renew(&target);
    assert_eq!(answer(&target), (None, 0, 0));
    assert_eq!(target.status().registrations_sent, 1);
}

/// A registration an activation's immediate one supersedes wasn't left
/// unanswered; one its renewal finds still outstanding was.
#[tokio::test(start_paused = true)]
async fn a_superseded_registration_is_not_unanswered() {
    let a = bbmd(1);
    let target = ForeignTarget::at(a);
    renew(&target);
    target.pass(Instant::now(), Cause::Change);
    target.registration_sent();
    assert_eq!(answer(&target), (None, 0, 0));
    renew(&target);
    assert_eq!(
        answer(&target),
        (Some(Bip6RegistrationAnswer::Unanswered), 1, 0)
    );
    target.result(a, SUCCESS);
    assert_eq!(answer(&target), (Some(Bip6RegistrationAnswer::Held), 0, 0));
}
