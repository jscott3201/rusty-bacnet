//! Clause 9 regressions for the waiting states, per-frame turnaround and
//! broadcast requests, run through the transport against a scripted peer
//! (#1608 U3, U4, U6).
//!
//! Each test states the rule it checks by clause; none encodes what the
//! transport did before the portable core.

use bacnet_mstp_codec::{BROADCAST_MAC, FrameType};
use tokio::time::{Duration, Instant};

use super::MstpConfig;
use super::test_support::{Bus, Sent, T_USAGE_TIMEOUT, config, encode};
use crate::port::TransportPort;

const T_NO_TOKEN: Duration = Duration::from_millis(500);
const T_SLOT: Duration = Duration::from_millis(10);
/// Slack for Tokio's millisecond timer wheel.
const TICK: Duration = Duration::from_millis(1);

/// A frame to `destination` whose data CRC is wrong: invalid for that
/// station once its header has passed (Clause 9.5.4).
fn bad_data_crc(destination: u8, source: u8) -> Vec<u8> {
    let mut wire = encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        destination,
        source,
        &[1, 0, 0x55],
    );
    let last = wire.len() - 1;
    wire[last] ^= 0x01;
    wire
}

/// A frame whose header CRC is wrong: invalid wherever it was going.
fn bad_header_crc(destination: u8, source: u8) -> Vec<u8> {
    let mut wire = encode(FrameType::TOKEN, destination, source, &[]);
    wire[7] ^= 0x01;
    wire
}

/// Station 3 in a ring with station 4, holding the token and waiting for
/// the reply to a Data Expecting Reply it sent to 4.
async fn waiting_for_reply() -> (Bus, Sent) {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.transport
        .send_unicast(&[1, 0x04, 0x10], &[4])
        .await
        .unwrap();
    bus.return_token(&token, 4).await;
    let request = bus.next().await;
    assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
    bus.until(request.end + bus.turnaround).await;
    (bus, request)
}

/// After dropping the token, a station hears nothing more and generates a
/// token in its slot after the frame that made it drop the token: proof it
/// went to IDLE rather than on with the token.
async fn expect_token_dropped(bus: &mut Bus, heard: Instant) {
    let slot = heard + T_NO_TOKEN + T_SLOT * 3;
    let next = bus.next().await;
    assert_eq!(
        next.frame_type,
        FrameType::POLL_FOR_MASTER,
        "the token was not dropped: {next:?}"
    );
    assert!(next.start >= slot, "{next:?} came before the slot");
    assert!(next.start <= slot + TICK);
}

/// U3, WAIT_FOR_REPLY. A valid frame that is not a reply for this station
/// (one for another station, or a type that is no reply) means another
/// token may be about: the station goes to IDLE and drops the token
/// (ReceivedUnexpectedFrame, Clause 9.5.6.4). It must not wait out
/// T_reply_timeout and then pass the token.
#[tokio::test(start_paused = true)]
async fn u3_unexpected_frame_in_wait_for_reply_drops_the_token() {
    for (frame_type, destination) in [
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2),
        (FrameType::TOKEN, 3),
        (FrameType::POLL_FOR_MASTER, 3),
    ] {
        let (mut bus, _request) = waiting_for_reply().await;
        let data: &[u8] = if frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            &[1, 0, 0x20]
        } else {
            &[]
        };
        let heard = bus.send(frame_type, destination, 4, data);
        expect_token_dropped(&mut bus, heard).await;
        bus.stop().await;
    }
}

/// U3, WAIT_FOR_REPLY. An invalid frame there ends the wait at once and
/// the station goes on to DONE_WITH_TOKEN, passing the token (InvalidFrame,
/// Clause 9.5.6.4), well before T_reply_timeout.
#[tokio::test(start_paused = true)]
async fn u3_invalid_frame_in_wait_for_reply_passes_the_token() {
    for invalid in [bad_data_crc(3, 4), bad_header_crc(3, 4)] {
        let (mut bus, _request) = waiting_for_reply().await;
        let heard = bus.send_raw(&invalid);
        let token = bus.next().await;
        assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
        assert!(token.start >= heard + bus.turnaround);
        assert!(token.start <= heard + bus.turnaround + TICK, "{token:?}");
        let invalid_frames = bus.transport.diagnostics().snapshot().mac.parser_errors;
        assert_eq!(invalid_frames, 1);
        bus.stop().await;
    }
}

/// Station 3, with no known successor, given the token: it polls 4.
async fn polling() -> (Bus, Sent) {
    let mut bus = Bus::start(config(3, 7)).await;
    bus.send(FrameType::TOKEN, 3, 6, &[]);
    let poll = bus.next().await;
    assert_eq!(
        (poll.frame_type, poll.destination),
        (FrameType::POLL_FOR_MASTER, 4)
    );
    (bus, poll)
}

/// U3, POLL_FOR_MASTER. A valid frame other than an answer to this station's
/// poll drops the token: IDLE (ReceivedUnexpectedFrame, Clause
/// 9.5.6.8). No further poll and no token pass follow.
#[tokio::test(start_paused = true)]
async fn u3_unexpected_frame_in_poll_for_master_drops_the_token() {
    for (frame_type, destination) in [
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, BROADCAST_MAC),
        (FrameType::REPLY_TO_POLL_FOR_MASTER, 2),
        (FrameType::TOKEN, 3),
    ] {
        let (mut bus, poll) = polling().await;
        bus.until(poll.end + bus.turnaround).await;
        let data: &[u8] = if frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            &[1, 0, 0x20]
        } else {
            &[]
        };
        let heard = bus.send(frame_type, destination, 6, data);
        expect_token_dropped(&mut bus, heard).await;
        bus.stop().await;
    }
}

/// U3, POLL_FOR_MASTER. An invalid frame counts as no answer to the poll,
/// at once (SendNextPFM, Clause 9.5.6.8): the next poll follows the invalid
/// frame by T_turnaround, not the poll by T_usage_timeout. The station
/// stays silent until then.
#[tokio::test(start_paused = true)]
async fn u3_invalid_frame_in_poll_for_master_polls_on() {
    for invalid in [bad_data_crc(3, 4), bad_header_crc(9, 4)] {
        let (mut bus, poll) = polling().await;
        let at = poll.end + Duration::from_millis(12);
        tokio::select! {
            sent = bus.next() => panic!("{sent:?} before the invalid frame"),
            () = tokio::time::sleep_until(at) => {}
        }
        let heard = bus.send_raw(&invalid);
        let next = bus.next().await;
        assert_eq!(
            (next.frame_type, next.destination),
            (FrameType::POLL_FOR_MASTER, 5)
        );
        assert!(next.start >= heard + bus.turnaround);
        assert!(next.start <= heard + bus.turnaround + TICK);
        assert!(next.start < poll.end + T_USAGE_TIMEOUT);
        bus.stop().await;
    }
}

/// U4. Every frame waits T_turnaround after the last activity on the line
/// before the driver goes on (Clause 9.5.5.1 step 1), and the station's own
/// transmission is such activity: it resets the silence timer octet by
/// octet. So each frame of one token use (two data frames, then the Token)
/// starts no sooner than T_turnaround after the previous one ended.
#[tokio::test(start_paused = true)]
async fn u4_turnaround_separates_every_frame() {
    let mut bus = Bus::start(MstpConfig {
        max_info_frames: 2,
        ..config(3, 4)
    })
    .await;
    let token = bus.join(4).await;
    bus.transport
        .send_unicast(&[1, 0, 0x01], &[4])
        .await
        .unwrap();
    bus.transport.send_broadcast(&[1, 0, 0x02]).await.unwrap();
    let mut last_end = bus.return_token(&token, 4).await;
    for expected in [
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        FrameType::TOKEN,
    ] {
        let sent = bus.next().await;
        assert_eq!(sent.frame_type, expected, "{sent:?}");
        assert!(
            sent.start >= last_end + bus.turnaround,
            "{sent:?} started {:?} after the last activity",
            sent.start - last_end
        );
        assert!(
            sent.start <= last_end + bus.turnaround + TICK,
            "{sent:?} started {:?} after the last activity",
            sent.start - last_end
        );
        last_end = sent.end;
    }
    bus.stop().await;
}

/// U6. A broadcast Data Expecting Reply is passed up like any other data
/// (BroadcastDataNeedingReply, Clause 9.5.6.2), with no reply channel: no
/// station answers a broadcast at the data link. The station stays in IDLE
/// and sends nothing for it.
#[tokio::test(start_paused = true)]
async fn u6_broadcast_data_expecting_reply_is_delivered() {
    let mut bus = Bus::start(config(3, 7)).await;
    let heard = bus.send(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        BROADCAST_MAC,
        4,
        &[1, 0x04, 0x77],
    );
    let npdu = tokio::time::timeout(Duration::from_millis(1), bus.npdus.recv())
        .await
        .expect("the broadcast request was not delivered")
        .unwrap();
    assert_eq!(npdu.npdu.as_ref(), &[1, 0x04, 0x77]);
    assert_eq!(npdu.source_mac.as_slice(), &[4]);
    assert!(npdu.link_layer_group);
    assert!(npdu.reply_tx.is_none());
    // Nothing answers it; the next frame is the station's own token
    // generation after a silence measured from the request.
    expect_token_dropped(&mut bus, heard).await;
    bus.stop().await;
}
