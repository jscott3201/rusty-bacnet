//! The frame tap on a running station (#1856): it times received frames as
//! the master node does, in paused Tokio time.

use bacnet_mstp_codec::FrameType;
use tokio::time::Duration;

use super::HOST_FRAME_ABORT_MS;
use super::test_support::{Bus, config, encode};
use crate::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};

/// The frames tapped so far that the station heard, in order.
fn heard(frames: &mut FrameTapReceiver) -> Vec<TappedFrame> {
    std::iter::from_fn(|| frames.try_recv())
        .filter(|frame| frame.direction == TapDirection::Inbound)
        .collect()
}

/// A frame whose last octet comes after a silence of exactly T_frame_abort
/// is taken by the node and tapped once. One whose last octet comes after a
/// longer silence is aborted by the node and not tapped, whole or in part,
/// though the silence is shorter than T_frame_abort plus that octet's time
/// on the wire. A whole frame after it is taken and tapped once.
#[tokio::test(start_paused = true)]
async fn the_tap_drops_a_frame_the_node_aborts_for_a_silence() {
    let mut bus = Bus::start(config(3, 127)).await;
    let mut frames = bus.transport.frame_tap().subscribe();
    let data = |last: u8| {
        encode(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            3,
            7,
            &[1, 0, last],
        )
    };
    let (kept, aborted, whole) = (data(0x41), data(0x42), data(0x43));
    let split = kept.len() - 1;
    assert!(Duration::from_millis(1) < bus.octet, "9600 baud");
    for (frame, gap, taken) in [
        (&kept, HOST_FRAME_ABORT_MS, true),
        (&aborted, HOST_FRAME_ABORT_MS + 1, false),
    ] {
        bus.send_raw(&frame[..split]);
        tokio::time::sleep(Duration::from_millis(gap)).await;
        bus.send_raw(&frame[split..]);
        let npdu = tokio::time::timeout(Duration::from_millis(5), bus.npdus.recv()).await;
        assert_eq!(npdu.is_ok(), taken, "gap {gap} ms");
    }
    bus.send_raw(&whole);
    assert_eq!(bus.npdus.recv().await.unwrap().npdu[..], [1, 0, 0x43]);

    let heard = heard(&mut frames);
    let octets: Vec<&[u8]> = heard.iter().map(|frame| &frame.bytes[..]).collect();
    assert_eq!(octets, [&kept[..], &whole[..]]);
    assert!(heard.iter().all(|frame| frame.peer == TapPeer::Mstp(7)));
    let counts = bus.transport.diagnostics().snapshot().mac;
    assert_eq!((counts.frames_received, counts.frame_aborts), (2, 1));
    bus.stop().await;
}

/// The node gives up a frame it is part way into when it starts to send,
/// since its receiver goes off, and so does the tap: the frame's head,
/// heard before the station's next frame started, and its tail, heard after,
/// are not tapped as a frame, though they arrive well within T_frame_abort.
#[tokio::test(start_paused = true)]
async fn the_tap_drops_a_frame_the_node_gives_up_to_send() {
    let mut bus = Bus::start(config(3, 127)).await;
    let mut frames = bus.transport.frame_tap().subscribe();
    let token = encode(FrameType::TOKEN, 3, 7, &[]);
    let cut = encode(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        3,
        7,
        &[1, 0, 0x44],
    );
    bus.send_raw(&token);
    let head = bus.send_raw(&cut[..4]);
    // Holding the token, the station sends once the line has been quiet
    // for T_turnaround, with the head part way in.
    let sent = bus.next().await;
    assert_eq!(sent.source, 3);
    // The tail is read once the station's frame is done.
    bus.send_raw(&cut[4..]);
    let read = sent.end + Duration::from_millis(5);
    assert!(read - head < Duration::from_millis(HOST_FRAME_ABORT_MS));
    bus.until(read).await;

    let heard = heard(&mut frames);
    let octets: Vec<&[u8]> = heard.iter().map(|frame| &frame.bytes[..]).collect();
    assert_eq!(octets, [&token[..]]);
    let counts = bus.transport.diagnostics().snapshot().mac;
    assert_eq!((counts.frames_received, counts.frame_aborts), (1, 1));
    assert!(bus.npdus.try_recv().is_err());
    bus.stop().await;
}
