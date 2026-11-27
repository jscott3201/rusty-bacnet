//! A Device recipient bound by its answer to the server's own Who-Is keeps
//! its notifications when another station claims its instance in an I-Am the
//! server did not ask for (#2825), until two checks a minute apart find its
//! old address failing while the claimant answers.

use super::*;
use crate::server::device_bindings::CHECK_HOLD_OFF;
use crate::server::remote_write_discovery_tests::{directed_who_is_sent, identity_reads};

/// Another station on the link, which claims Device 9 unasked.
const STRANGER: [u8; 6] = [10, 0, 0, 66, 0xBA, 0xC0];

/// The invoke ID of each confirmed event notification sent to `mac`.
fn confirmed_ids(h: &Harness, mac: &[u8]) -> Vec<u8> {
    let log = h.server.test_network().transport().sent();
    log.frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == mac)
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request)
                if request.service_choice
                    == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION =>
            {
                Some(request.invoke_id)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_strangers_i_am_does_not_move_a_device_recipient() {
    let mut h = start(true).await;
    tokio::join!(h.write_local(85.0), async {
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    h.settle().await;
    let [first] = confirmed_ids(&h, &DEVICE_9)[..] else {
        panic!("one confirmed notification to Device 9");
    };
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id: first,
        service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
    });
    deliver(&h, &ack, &DEVICE_9, None).await;
    h.settle().await;

    deliver(&h, &i_am(9), &STRANGER, None).await;
    h.settle().await;
    h.write_local(50.0).await;
    h.settle().await;
    assert_eq!(confirmed_ids(&h, &STRANGER), []);
    assert_eq!(confirmed_ids(&h, &DEVICE_9).len(), 2);
    h.server.stop().await.unwrap();
}

/// Answer the Who-Is a check sends, once it has gone out: straight to
/// STRANGER for the first check, broadcast on this network for the second.
async fn stranger_answers(h: &Harness, broadcast: bool) {
    let mut asked = Vec::new();
    until("the check's Who-Is", || {
        let directed = directed_who_is_sent(h)
            .into_iter()
            .map(|(mac, (_, who_is))| (Some(mac), who_is));
        asked.extend(directed);
        asked.extend(who_is_sent(h).into_iter().map(|(_, who_is)| (None, who_is)));
        !asked.is_empty()
    })
    .await;
    let to = (!broadcast).then(|| STRANGER.to_vec());
    assert_eq!(asked, [(to, targeted(9))]);
    deliver(h, &i_am(9), &STRANGER, None).await;
}

/// Device 9 really moved to STRANGER's address: its old address is silent
/// through both checks' reads. Every confirmed notification before the
/// move goes to the old address, none to the new one; the one after goes
/// there.
#[tokio::test(start_paused = true)]
async fn a_device_recipient_that_moved_gets_nothing_there_before_the_move() {
    let mut h = start(true).await;
    tokio::join!(h.write_local(85.0), async {
        assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
        deliver(&h, &i_am(9), &DEVICE_9, None).await;
    });
    h.settle().await;
    let [first] = confirmed_ids(&h, &DEVICE_9)[..] else {
        panic!("one confirmed notification to Device 9");
    };
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id: first,
        service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
    });
    deliver(&h, &ack, &DEVICE_9, None).await;
    h.settle().await;

    deliver(&h, &i_am(9), &STRANGER, None).await;
    stranger_answers(&h, false).await;
    h.write_local(50.0).await;
    // The first read times out 12 s on; the second check comes a minute
    // after that, and its read times out 12 s later.
    tokio::time::sleep(Duration::from_secs(12) + CHECK_HOLD_OFF - Duration::from_secs(2)).await;
    stranger_answers(&h, true).await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    let bound_at = |h: &Harness| {
        let now = crate::runtime_clock::now();
        let table = h.server.device_bindings.try_read().unwrap();
        table.resolve_at(&device(9), now, |_| false)
    };
    until("the move", || {
        assert_eq!(confirmed_ids(&h, &STRANGER), []);
        matches!(bound_at(&h), DeviceResolution::ResolvedLocal { peer_mac, .. }
            if peer_mac.as_slice() == STRANGER)
    })
    .await;
    let reads = identity_reads(&h);
    assert!(reads.iter().all(|(mac, ..)| mac.as_slice() == DEVICE_9));
    assert_eq!(reads.len(), 8, "two reads, each tried four times");
    let to_old = confirmed_ids(&h, &DEVICE_9).len();
    assert_eq!(to_old, 1 + 4, "the alarm, then the clear tried four times");
    h.write_local(85.0).await;
    h.settle().await;
    assert_eq!(confirmed_ids(&h, &STRANGER).len(), 1);
    h.server.stop().await.unwrap();
}
