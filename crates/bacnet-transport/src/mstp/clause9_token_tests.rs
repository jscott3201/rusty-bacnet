//! Clause 9 regressions for token maintenance and token generation, run
//! through the transport against a scripted peer (#1608 U1, U2, U5).
//!
//! Each test states the rule it checks by clause; none encodes what the
//! transport did before the portable core.

use bacnet_mstp_codec::FrameType;
use tokio::time::{Duration, Instant};

use super::test_support::{config, Bus, T_USAGE_TIMEOUT};

const T_NO_TOKEN: Duration = Duration::from_millis(500);
const T_SLOT: Duration = Duration::from_millis(10);
const T_USAGE_DELAY: Duration = Duration::from_millis(15);
/// Slack for Tokio's millisecond timer wheel.
const TICK: Duration = Duration::from_millis(1);

/// U1. In DONE_WITH_TOKEN a master passes the token while TokenCount is
/// below Npoll (50) and polls for a master once it reaches it (SendToken and
/// SendMaintenancePFM, Clause 9.5.6.5). ReceivedReplyToPFM sets TokenCount
/// to zero (Clause 9.5.6.8), so after joining a ring the station passes the
/// token 50 times, and answers the 51st token with a `PollForMaster` to the
/// first address after it.
#[tokio::test(start_paused = true)]
async fn u1_maintenance_poll_follows_npoll_token_passes() {
    // Station 3, successor 5: address 4 lies between them.
    let mut bus = Bus::start(config(3, 7)).await;
    let mut token = bus.join(5).await;
    for pass in 1..=50 {
        bus.return_token(&token, 5).await;
        token = bus.next().await;
        assert_eq!(
            (token.frame_type, token.destination),
            (FrameType::TOKEN, 5),
            "token use {pass} must pass the token"
        );
    }
    bus.return_token(&token, 5).await;
    let poll = bus.next().await;
    assert_eq!(
        (poll.frame_type, poll.destination),
        (FrameType::POLL_FOR_MASTER, 4),
        "the 51st token use polls the gap"
    );
    // Nobody answers: DoneWithPFM passes the token to the known successor.
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 5));
    assert!(token.start >= poll.end + T_USAGE_TIMEOUT);
    bus.stop().await;
}

/// U2. In POLL_FOR_MASTER the station waits T_usage_timeout for an answer
/// to its poll (Clause 9.5.6.8, SendNextPFM and the others). A polled
/// master may take up to T_usage_delay (15 ms) to answer (Clause 9.5.3), so
/// an answer that starts 15 ms after the poll makes it the successor, and
/// without one the next poll waits the full T_usage_timeout (the host's
/// 35 ms).
#[tokio::test(start_paused = true)]
async fn u2_poll_for_master_waits_the_usage_timeout() {
    let mut bus = Bus::start(config(3, 7)).await;
    bus.send(FrameType::TOKEN, 3, 6, &[]);

    // No answer from 4: the next poll comes T_usage_timeout after the first.
    let first = bus.next().await;
    assert_eq!(
        (first.frame_type, first.destination),
        (FrameType::POLL_FOR_MASTER, 4)
    );
    let second = bus.next().await;
    assert_eq!(
        (second.frame_type, second.destination),
        (FrameType::POLL_FOR_MASTER, 5)
    );
    assert!(
        second.start >= first.end + T_USAGE_TIMEOUT,
        "{first:?} {second:?}"
    );
    assert!(second.start <= first.end + T_USAGE_TIMEOUT + TICK);

    // Station 5 answers as late as T_usage_delay allows.
    bus.until(second.end + T_USAGE_DELAY).await;
    let answered = bus.send(FrameType::REPLY_TO_POLL_FOR_MASTER, 3, 5, &[]);
    let token = bus.next().await;
    assert_eq!(
        (token.frame_type, token.destination),
        (FrameType::TOKEN, 5),
        "a lawful late answer names the successor"
    );
    assert!(token.start >= answered + bus.turnaround);
    bus.stop().await;
}

/// U5, the slot. With no traffic, a master leaves IDLE after T_no_token
/// (LostToken, Clause 9.5.6.2) and generates a token once the silence
/// reaches T_no_token + T_slot × TS (GenerateToken, Clause 9.5.6.7), with
/// a `PollForMaster` to TS + 1. The slot is measured from the last activity
/// on the line, here the moment the station started.
#[tokio::test(start_paused = true)]
async fn u5_token_generation_uses_the_stations_own_slot() {
    for station in [0u8, 3, 7] {
        let started = Instant::now();
        let mut bus = Bus::start(config(station, 7)).await;
        let poll = bus.next().await;
        let slot = started + T_NO_TOKEN + T_SLOT * u32::from(station);
        assert_eq!(
            poll.frame_type,
            FrameType::POLL_FOR_MASTER,
            "station {station}"
        );
        assert_eq!(poll.destination, (station + 1) % 8);
        assert!(
            poll.start >= slot,
            "station {station}: {poll:?} before its slot"
        );
        assert!(
            poll.start <= slot + TICK,
            "station {station}: {poll:?} after its slot"
        );
        bus.stop().await;
    }
}

/// U5, back to IDLE. A frame heard while waiting in NO_TOKEN shows that
/// another station is active (SawFrame, Clause 9.5.6.7): the station returns
/// to IDLE instead of generating, and its next slot is measured from that
/// frame. Station 3's slot is at 530 ms; a frame at 505 ms moves it to
/// 505 + 530 ms.
#[tokio::test(start_paused = true)]
async fn u5_traffic_in_no_token_returns_to_idle() {
    let started = Instant::now();
    let mut bus = Bus::start(config(3, 7)).await;
    bus.until(started + T_NO_TOKEN + Duration::from_millis(5))
        .await;
    let transitions = || bus.transport.diagnostics().snapshot().mac.state_transitions;
    assert_eq!(transitions(), 1, "IDLE to NO_TOKEN");
    let heard = bus.send(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2, 1, &[1, 0, 9]);
    tokio::time::sleep(TICK).await;
    assert_eq!(
        transitions(),
        2,
        "the frame did not send NO_TOKEN back to IDLE"
    );
    let poll = bus.next().await;
    assert_eq!(poll.frame_type, FrameType::POLL_FOR_MASTER);
    let slot = heard + T_NO_TOKEN + T_SLOT * 3;
    assert!(
        poll.start >= slot,
        "generated at {:?} after start, before the new slot",
        poll.start - started
    );
    assert!(poll.start <= slot + TICK);
    bus.stop().await;
}
