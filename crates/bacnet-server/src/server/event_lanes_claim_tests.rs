//! A Device recipient bound by its answer to the server's own Who-Is keeps
//! its notifications when another station claims its instance in an I-Am the
//! server did not ask for (#2825).

use super::*;

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
