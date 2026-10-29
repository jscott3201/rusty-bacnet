//! A USB adapter that hands the station what it receives late (#1655).
//!
//! An FTDI part holds received octets until its latency timer runs out,
//! 16 ms by default, and the host picks them up at the next 1 ms USB frame.
//! Here every frame a peer begins on the wire at `t` reaches the station
//! whole at `t` + [`RECEIVE_LATENCY`], in one read, while the station's own
//! frames end on time. Each peer answers as late as Clause 9.5.3 lets it:
//! a token used, or a poll answered, after T_usage_delay, and a request
//! answered after T_reply_delay.
//!
//! With the host's timeouts (T_usage_timeout 35 ms, T_reply_timeout
//! 300 ms) the station hears every answer in time. Each test then runs the
//! same exchange with the shortest timeouts 9.5.3 allows (20 ms and 255 ms),
//! which the transport used before, and shows the station giving up on an
//! answer that is already on the wire.

use bacnet_mstp_codec::FrameType;
use tokio::time::{Duration, Instant};

use super::MstpConfig;
use super::test_support::{Bus, Sent, T_REPLY_TIMEOUT, T_USAGE_TIMEOUT, config};
use crate::port::TransportPort;

/// How late the station hears a frame: FTDI's default latency timer plus one
/// USB frame.
const RECEIVE_LATENCY: Duration = Duration::from_millis(17);
/// The longest a station may take to use the token or answer a poll.
const T_USAGE_DELAY: Duration = Duration::from_millis(15);
/// The longest a station may take to begin answering a request.
const T_REPLY_DELAY: Duration = Duration::from_millis(250);
/// The shortest T_usage_timeout and T_reply_timeout Clause 9.5.3 allows.
const SHORTEST_USAGE_TIMEOUT: Duration = Duration::from_millis(20);
const SHORTEST_REPLY_TIMEOUT: Duration = Duration::from_millis(255);

/// `config` with the shortest timeouts, as the transport had before #1655.
fn shortest(config: MstpConfig) -> MstpConfig {
    MstpConfig {
        usage_timeout: SHORTEST_USAGE_TIMEOUT,
        reply_timeout: SHORTEST_REPLY_TIMEOUT,
        ..config
    }
}

/// A peer, `source`, begins `frame_type` on the wire `after` the end of the
/// station's frame `sent`; the adapter hands it to the station
/// [`RECEIVE_LATENCY`] later. Returns when the station read it.
async fn late(
    bus: &Bus,
    sent: &Sent,
    after: Duration,
    frame_type: FrameType,
    source: u8,
    data: &[u8],
) -> Instant {
    bus.until(sent.end + after + RECEIVE_LATENCY).await;
    bus.send(frame_type, bus.station, source, data)
}

/// The host's timeouts are the longest Clause 9.5.3 allows, and the late
/// answers below fit inside them only with the adapter's latency counted.
#[test]
fn the_host_timeouts_are_the_longest_allowed() {
    assert_eq!(T_USAGE_TIMEOUT, Duration::from_millis(35));
    assert_eq!(T_REPLY_TIMEOUT, Duration::from_millis(300));
    let defaults = MstpConfig::default();
    assert_eq!(
        (defaults.usage_timeout, defaults.reply_timeout),
        (T_USAGE_TIMEOUT, T_REPLY_TIMEOUT)
    );
    let core = defaults.core_config().unwrap();
    assert_eq!(core.t_usage_timeout, bacnet_mstp_core::MAX_USAGE_TIMEOUT);
    assert_eq!(core.t_reply_timeout, bacnet_mstp_core::MAX_REPLY_TIMEOUT);
    assert_eq!(
        Duration::from(bacnet_mstp_core::MIN_USAGE_TIMEOUT),
        SHORTEST_USAGE_TIMEOUT
    );
    assert_eq!(
        Duration::from(bacnet_mstp_core::MIN_REPLY_TIMEOUT),
        SHORTEST_REPLY_TIMEOUT
    );
    assert!(T_USAGE_DELAY + RECEIVE_LATENCY < T_USAGE_TIMEOUT);
    assert!(T_USAGE_DELAY + RECEIVE_LATENCY > SHORTEST_USAGE_TIMEOUT);
    assert!(T_REPLY_DELAY + RECEIVE_LATENCY < T_REPLY_TIMEOUT);
    assert!(T_REPLY_DELAY + RECEIVE_LATENCY > SHORTEST_REPLY_TIMEOUT);
}

/// Both timeouts are configurable within Clause 9.5.3's bounds, and
/// `validate` names the field it refuses.
#[test]
fn the_timeouts_are_held_to_clause_9_5_3() {
    let ms = Duration::from_millis;
    for (usage, reply) in [(ms(20), ms(255)), (ms(35), ms(300)), (ms(27), ms(280))] {
        let config = MstpConfig {
            usage_timeout: usage,
            reply_timeout: reply,
            ..config(3, 4)
        };
        let core = config.core_config().unwrap();
        assert_eq!(Duration::from(core.t_usage_timeout), usage);
        assert_eq!(Duration::from(core.t_reply_timeout), reply);
    }
    for (usage, reply, field) in [
        (ms(19), ms(300), "usage_timeout"),
        (Duration::from_micros(35_001), ms(300), "usage_timeout"),
        (ms(35), ms(254), "reply_timeout"),
        (ms(35), Duration::from_micros(300_001), "reply_timeout"),
    ] {
        let config = MstpConfig {
            usage_timeout: usage,
            reply_timeout: reply,
            ..config(3, 4)
        };
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains(field), "{error}");
        assert!(error.contains("9.5.3"), "{error}");
    }
}

/// PASS_TOKEN (Clause 9.5.6.6). The successor uses the token T_usage_delay
/// after it, and the station hears that 32 ms after its Token ended. With
/// 35 ms it has waited: the successor's Token is the next thing it acts on,
/// and it never sent its own again. With 20 ms it sends the token a second
/// time 20 ms after the first, while the successor is already transmitting.
#[tokio::test(start_paused = true)]
async fn a_successor_heard_late_through_the_adapter_still_has_the_token() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    let heard = late(&bus, &token, T_USAGE_DELAY, FrameType::TOKEN, 4, &[]).await;
    let next = bus.next().await;
    assert_eq!((next.frame_type, next.destination), (FrameType::TOKEN, 4));
    assert!(
        next.start >= heard + bus.turnaround,
        "sent before it heard the successor: {next:?}"
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.token_retries, 0);
    assert_eq!(counts.mac.token_regenerations, 0);
    bus.stop().await;

    let mut bus = Bus::start(shortest(config(3, 4))).await;
    let token = bus.join(4).await;
    let heard = late(&bus, &token, T_USAGE_DELAY, FrameType::TOKEN, 4, &[]).await;
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::TOKEN, 4));
    assert!(
        retry.start >= token.end + SHORTEST_USAGE_TIMEOUT,
        "{retry:?}"
    );
    assert!(retry.start < heard, "{retry:?}");
    assert_eq!(bus.transport.diagnostics().snapshot().mac.token_retries, 1);
    bus.stop().await;
}

/// POLL_FOR_MASTER (Clause 9.5.6.8). Station 4 answers the poll
/// T_usage_delay after it, and the station hears the answer 32 ms after its
/// poll. With 35 ms station 4 becomes the successor and gets the token. With
/// 20 ms the station has already given up on 4 and polled 5.
#[tokio::test(start_paused = true)]
async fn a_poll_answered_late_through_the_adapter_finds_the_successor() {
    for shortest_timeouts in [false, true] {
        let mut config = config(3, 7);
        if shortest_timeouts {
            config = shortest(config);
        }
        let mut bus = Bus::start(config).await;
        bus.send(FrameType::TOKEN, 3, 6, &[]);
        let poll = bus.next().await;
        assert_eq!(
            (poll.frame_type, poll.destination),
            (FrameType::POLL_FOR_MASTER, 4)
        );
        let heard = late(
            &bus,
            &poll,
            T_USAGE_DELAY,
            FrameType::REPLY_TO_POLL_FOR_MASTER,
            4,
            &[],
        )
        .await;
        let next = bus.next().await;
        if shortest_timeouts {
            assert_eq!(
                (next.frame_type, next.destination),
                (FrameType::POLL_FOR_MASTER, 5),
                "{next:?}"
            );
            assert!(next.start >= poll.end + SHORTEST_USAGE_TIMEOUT, "{next:?}");
            assert!(next.start < heard, "{next:?}");
        } else {
            assert_eq!(
                (next.frame_type, next.destination),
                (FrameType::TOKEN, 4),
                "{next:?}"
            );
            assert!(next.start >= heard + bus.turnaround, "{next:?}");
        }
        bus.stop().await;
    }
}

/// WAIT_FOR_REPLY (Clause 9.5.6.4). Station 4 begins its answer to the
/// station's request T_reply_delay after it, and the station hears it
/// 267 ms after its request. With 300 ms the answer goes up and the station
/// passes the token after it, with no reply timeout. With 255 ms the
/// station has stopped waiting and passed the token before the answer
/// reached it.
#[tokio::test(start_paused = true)]
async fn a_reply_heard_late_through_the_adapter_is_taken() {
    for shortest_timeouts in [false, true] {
        let mut config = config(3, 4);
        if shortest_timeouts {
            config = shortest(config);
        }
        let mut bus = Bus::start(config).await;
        let token = bus.join(4).await;
        bus.transport
            .send_unicast(&[1, 4, 0x21], &[4])
            .await
            .unwrap();
        bus.return_token(&token, 4).await;
        let request = bus.next().await;
        assert_eq!(request.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
        let answer = [1, 0, 0x22];
        let heard = late(
            &bus,
            &request,
            T_REPLY_DELAY,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            4,
            &answer,
        )
        .await;
        let pass = bus.next().await;
        assert_eq!((pass.frame_type, pass.destination), (FrameType::TOKEN, 4));
        let counts = bus.transport.diagnostics().snapshot();
        if shortest_timeouts {
            assert!(
                pass.start >= request.end + SHORTEST_REPLY_TIMEOUT,
                "{pass:?}"
            );
            assert!(pass.start < heard, "{pass:?}");
            assert_eq!(counts.wait_for_reply_timeouts, 1);
        } else {
            assert!(pass.start >= heard + bus.turnaround, "{pass:?}");
            assert_eq!(counts.wait_for_reply_timeouts, 0);
            let delivered = tokio::time::timeout(Duration::from_millis(1), bus.npdus.recv())
                .await
                .expect("the answer went up")
                .unwrap();
            assert_eq!(delivered.npdu.as_ref(), &answer);
            assert_eq!(delivered.source_mac.as_slice(), &[4]);
        }
        bus.stop().await;
    }
}
