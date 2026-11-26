//! Floods of forged I-Ams against bound devices that keep answering
//! (#2825): what the checks they cue cost on the wire.
use super::*;
use crate::server::device_bindings::MAX_CHECKS;

/// Forged I-Ams for Device 9, one every five seconds for an hour, each from
/// an address of its own, while the device names itself to every read: one
/// check a minute, each a Who-Is straight to the forger and one read of the
/// device, none broadcast, and the binding never moves.
#[tokio::test(start_paused = true)]
async fn a_flood_against_one_device_costs_two_frames_a_minute() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let (mut asked, mut read) = (0, 0);
    for forged in 0..720u16 {
        let [high, low] = forged.to_be_bytes();
        let stranger = [10, 1, high, low, 0xBA, 0xC0];
        deliver(&h, &i_am(9), &stranger, None).await;
        h.settle().await;
        let sent = checked(&h);
        if !sent.is_empty() {
            assert_eq!(sent, [stranger.to_vec()]);
            asked += 1;
        }
        for (mac, invoke_id, object) in identity_reads(&h) {
            assert_eq!((mac.as_slice(), object), (&PEER[..], any_device()));
            read += 1;
            h.respond(names(invoke_id, 9)).await;
        }
        tokio::time::sleep(Duration::from_secs(5) - Duration::from_millis(1)).await;
    }
    assert_eq!((asked, read), (60, 60), "two frames a minute");
    assert!(who_is_sent(&h).is_empty(), "nothing broadcast");
    assert_eq!(bound(&h).await, PEER);
    write_answered(&h).await;
}

/// Forged I-Ams for 80 bound devices, one each every five seconds for three
/// minutes, each device naming itself to every read: at most
/// [`MAX_CHECKS`] devices are checked a minute, each costing a Who-Is and
/// a read, so the flood costs 128 frames a minute however many devices it
/// names. A cue finding every place held starts nothing.
#[tokio::test(start_paused = true)]
async fn a_flood_across_many_devices_costs_at_most_two_frames_a_check() {
    const DEVICES: u8 = 80;
    let station = |index: u8| [10, 0, 2, index, 0xBA, 0xC0];
    let h = start_unbound().await;
    {
        let mut table = h.server.device_bindings.write().await;
        for index in 0..DEVICES {
            let instance = 100 + u32::from(index);
            let now = runtime_clock::now();
            table.bind_found_at(device(instance), &station(index), None, now, |_| false);
        }
    }
    let (mut asked, mut read) = (0usize, 0usize);
    for round in 0..36u16 {
        let [high, low] = round.to_be_bytes();
        for index in 0..DEVICES {
            let forger = [10, 1, high, low ^ index, 0xBA, 0xC1];
            deliver(&h, &i_am(100 + u32::from(index)), &forger, None).await;
        }
        loop {
            h.settle().await;
            asked += directed_who_is_sent(&h).len();
            let reads = identity_reads(&h);
            if reads.is_empty() {
                break;
            }
            for (mac, invoke_id, object) in reads {
                assert_eq!(object, any_device());
                let index = mac[3];
                assert_eq!(mac, station(index));
                read += 1;
                let answer = names(invoke_id, 100 + u32::from(index));
                deliver(&h, &answer, &mac, None).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(5) - Duration::from_millis(1)).await;
    }
    // Rounds at 0, 60 and 120 seconds each check the cap's worth.
    assert_eq!((asked, read), (3 * MAX_CHECKS, 3 * MAX_CHECKS));
    assert!(who_is_sent(&h).is_empty(), "nothing broadcast");
}
