//! Echo and transmit completion for each kind of port: what the transport
//! does with the octets a port hears while it sends, and when it reports a
//! frame complete to the core.

use bacnet_mstp_codec::{decode_frame, FrameType};
use tokio::time::Duration;

use super::test_support::{config, encode, Bus, DrainMode, Port, T_USAGE_TIMEOUT};
use super::{Echo, LoopbackSerial, MstpTransport, SerialPort};
use crate::port::TransportPort;

const TICK: Duration = Duration::from_millis(1);

pub(super) const ECHOING: Port = Port {
    echo: Echo::full(),
    echoes: true,
    drain: DrainMode::Immediate,
};

/// The port declares an echo, and the test scripts what comes back.
pub(super) const SCRIPTED_ECHO: Port = Port {
    echo: Echo::full(),
    echoes: false,
    drain: DrainMode::Immediate,
};

/// A station whose port hears its own frames still treats its own Token as
/// its own: with the successor silent, PASS_TOKEN retries the token after
/// T_usage_timeout (RetrySendToken, Clause 9.5.6.6). Had the echo reached the
/// core as line activity, its eight octets would read as the successor using
/// the token (SawTokenUser) and there would be no retry.
#[tokio::test(start_paused = true)]
async fn an_echoing_port_does_not_hear_its_own_token_as_the_successor() {
    let mut bus = Bus::start_with(config(3, 4), ECHOING).await;
    let token = bus.join(4).await;
    let retry = bus.next().await;
    assert_eq!((retry.frame_type, retry.destination), (FrameType::TOKEN, 4));
    assert!(retry.start >= token.end + T_USAGE_TIMEOUT);
    assert!(retry.start <= token.end + T_USAGE_TIMEOUT + TICK);
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.token_retries, 1);
    assert_eq!((counts.echo_mismatches, counts.echo_missing), (0, 0));
    bus.stop().await;
}

/// Octets read after the echo, in the same read, are line activity after
/// the frame: the successor's Token right behind the echo shows it took the
/// token, and the station, given it back, passes it on again.
#[tokio::test(start_paused = true)]
async fn octets_after_the_echo_in_one_read_are_line_activity() {
    let mut bus = Bus::start_with(config(3, 4), SCRIPTED_ECHO).await;
    // Joining: echo each poll and the Token by hand.
    bus.send(FrameType::TOKEN, 3, 4, &[]);
    let poll = bus.next().await;
    assert_eq!(poll.destination, 4);
    let mut chunk = encode(poll.frame_type, poll.destination, poll.source, &[]);
    chunk.extend(encode(FrameType::REPLY_TO_POLL_FOR_MASTER, 3, 4, &[]));
    bus.send_raw(&chunk);
    let token = bus.next().await;
    assert_eq!((token.frame_type, token.destination), (FrameType::TOKEN, 4));
    let mut chunk = encode(FrameType::TOKEN, 4, 3, &[]);
    chunk.extend(encode(FrameType::TOKEN, 3, 4, &[]));
    bus.send_raw(&chunk);
    let again = bus.next().await;
    assert_eq!((again.frame_type, again.destination), (FrameType::TOKEN, 4));
    assert!(
        again.start < token.end + T_USAGE_TIMEOUT,
        "a retry: {again:?}"
    );
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.token_retries, 0);
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!((counts.echo_mismatches, counts.echo_missing), (0, 0));
    bus.stop().await;
}

/// An echo that differs from what was sent (a collision, or noise) is not
/// dropped: the station cannot tell it from a peer's octets, so all of it
/// reaches the core, in order, and is counted. Here the core finds an
/// invalid frame, which in POLL_FOR_MASTER counts as no answer at once
/// (SendNextPFM, Clause 9.5.6.8): the next poll does not wait for
/// T_usage_timeout.
#[tokio::test(start_paused = true)]
async fn a_corrupted_echo_is_fed_and_counted() {
    let mut bus = Bus::start_with(config(3, 4), SCRIPTED_ECHO).await;
    bus.send(FrameType::TOKEN, 3, 4, &[]);
    let poll = bus.next().await;
    let mut echo = encode(poll.frame_type, poll.destination, poll.source, &[]);
    echo[4] ^= 0x40;
    bus.until(poll.end + TICK).await;
    let heard = bus.send_raw(&echo);
    let next = bus.next().await;
    assert_eq!(
        (next.frame_type, next.destination),
        (FrameType::POLL_FOR_MASTER, 0)
    );
    assert!(next.start >= heard + bus.turnaround);
    assert!(next.start < poll.end + T_USAGE_TIMEOUT, "{next:?}");
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.echo_mismatches, 1);
    assert_eq!(
        counts.mac.parser_errors, 1,
        "the octets did not reach the core"
    );
    bus.stop().await;
}

/// A port that declares an echo but returns none costs each frame the echo
/// grace; it is counted, and the station still runs.
#[tokio::test(start_paused = true)]
async fn a_missing_echo_is_counted_after_the_grace() {
    let mut bus = Bus::start_with(config(3, 4), SCRIPTED_ECHO).await;
    bus.send(FrameType::TOKEN, 3, 4, &[]);
    let poll = bus.next().await;
    let next = bus.next().await;
    assert_eq!(next.frame_type, FrameType::POLL_FOR_MASTER);
    assert!(next.start >= poll.end + super::DEFAULT_ECHO_GRACE);
    let counts = bus.transport.diagnostics().snapshot();
    assert!(counts.echo_missing >= 1);
    bus.stop().await;
}

/// A write error aborts the frame: nothing is counted as sent, and the core
/// goes back to IDLE.
#[tokio::test(start_paused = true)]
async fn a_write_error_aborts_the_frame() {
    let mut bus = Bus::start(config(3, 4)).await;
    let token = bus.join(4).await;
    bus.fail_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    bus.transport.send_unicast(&[1, 4, 1], &[4]).await.unwrap();
    bus.return_token(&token, 4).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.serial_write_errors, 1);
    assert_eq!(counts.der_tx, 0);
    // The poll and the Token of joining; the failed frame is not one.
    assert_eq!(counts.mac.frames_transmitted, 2);
    bus.stop().await;
}

/// The same on [`LoopbackSerial`] built with [`Echo::full`]: a station
/// generating a token polls address after address, each poll's echo
/// dropped. Fed to the core, its own poll heard in POLL_FOR_MASTER would be
/// an unexpected frame and end the search (Clause 9.5.6.8).
#[tokio::test(start_paused = true)]
async fn loopback_with_echo_polls_on_past_its_own_echo() {
    let (serial, peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(serial.with_echo(Echo::full()), config(3, 7));
    let _npdus = transport.start().await.unwrap();
    let mut buf = [0u8; 64];
    for destination in [4, 5, 6] {
        let count = peer.read(&mut buf).await.unwrap();
        let (frame, _) = decode_frame(&buf[..count]).unwrap();
        assert_eq!(
            (frame.header.frame_type, frame.header.destination),
            (FrameType::POLL_FOR_MASTER, destination)
        );
    }
    let counts = transport.diagnostics().snapshot();
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.echo_missing, 0);
    transport.stop().await.unwrap();
}
