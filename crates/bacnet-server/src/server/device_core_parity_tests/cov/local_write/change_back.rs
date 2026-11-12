//! A change back before the first change's report has gone (#2508). The
//! server and bacnet-device-core's `Reporter` measure a change from the
//! newest observation decided for the subscription, a report still to go
//! when there is one, so both end with the current value. The `Reporter`,
//! which queues its reports, sends the newer in the older's place; the
//! server, which sends each at once, sends both. The parity corpus compares
//! each step's notifications as sets, so this case pins each side's own.
use super::*;
use bacnet_device_core::write::local::LocalChange;
use bacnet_objects::traits::LocalInputValue;

/// AI-1's Present_Value on the server now.
async fn present_value(cov: &CovParity) -> PropertyValue {
    let db = cov.parity.server.database().read().await;
    let object = db.get(&ai()).expect("AI-1");
    object.read_property(PV, None).unwrap()
}

/// The Present_Value an UnconfirmedCOVNotification APDU carries.
fn notified(apdu: &[u8]) -> PropertyValue {
    let Ok(Apdu::UnconfirmedRequest(request)) =
        bacnet_encoding::apdu::decode_apdu(Bytes::copy_from_slice(apdu))
    else {
        panic!("an unconfirmed notification: {apdu:02X?}");
    };
    let notification =
        bacnet_services::cov::COVNotificationRequest::decode(&request.service_request).unwrap();
    let value = &notification.list_of_values[0];
    bacnet_encoding::primitives::decode_application_value(&value.value, 0)
        .unwrap()
        .0
}

/// AI-1 to 25.0 and straight back, on each side at one instant: the server
/// sends both values, the `Reporter` the second alone, and the last
/// notification of each is the same octets.
#[tokio::test(start_paused = true)]
async fn a_change_back_ends_alike_on_both_sides() {
    let mut cov = CovParity::start_local().await;
    let before = present_value(&cov).await;
    let values = [PropertyValue::Real(25.0), before.clone()];
    let now = runtime_clock::core_now();
    for value in values.clone() {
        let server = &cov.parity.server;
        server.set_present_value_local(&ai(), value).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    let server: Vec<_> = cov
        .parity
        .sent
        .take()
        .iter()
        .map(|frame| frame.decode_npdu().payload.to_vec())
        .collect();
    let parity = &mut cov.parity;
    for value in values.clone() {
        let change = LocalChange::Input(LocalInputValue::present_value(value));
        let reporter = &mut parity.reporter;
        reporter
            .apply_local(&mut parity.db, ai(), change, now)
            .unwrap();
    }
    let mut core = Vec::new();
    loop {
        let mut out = BytesMut::new();
        let reporter = &mut cov.parity.reporter;
        let Some(sent) = reporter.pop(&NoBindings, &mut cov.ids, now, &mut out) else {
            break;
        };
        reporter.delivered(sent.token, Delivery::Sent, &mut cov.ids, now);
        core.push(out.to_vec());
    }
    let pvs = |apdus: &[Vec<u8>]| apdus.iter().map(|apdu| notified(apdu)).collect::<Vec<_>>();
    assert_eq!(pvs(&server), values, "the server sends both");
    assert_eq!(pvs(&core), [before], "the core sends the newer alone");
    assert_eq!(server.last(), core.last(), "and both end alike");
    cov.stop().await;
}

/// The server, too, can be asked for a change back while the first
/// change's send is still under way: a second change made meanwhile is
/// measured from that report, so it is sent rather than lost. Before #2508
/// it was measured from the delivered value, found nothing, and the
/// subscriber was left with the first change's value.
#[tokio::test(start_paused = true)]
async fn the_server_sends_a_change_back_made_while_a_send_is_under_way() {
    let cov = CovParity::start_local().await;
    let before = present_value(&cov).await;
    cov.link.block_next_send();
    let server = &cov.parity.server;
    let link = &cov.link;
    let ai1 = ai();
    let first = server.set_present_value_local(&ai1, PropertyValue::Real(25.0));
    let back = async {
        // The first change's notification is held in the transport.
        link.wait_blocked().await;
        let back = server.set_present_value_local(&ai1, before.clone()).await;
        link.release_sends(1);
        back
    };
    let (first, back) = tokio::join!(first, back);
    first.unwrap();
    back.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let sent: Vec<_> = cov
        .parity
        .sent
        .take()
        .iter()
        .map(|frame| notified(&frame.decode_npdu().payload))
        .collect();
    assert_eq!(sent, [PropertyValue::Real(25.0), before]);
    cov.stop().await;
}

/// A report the server couldn't send, that a later change was measured
/// from, has that change looked at again: the subscriber still hears 25.0.
/// The later change, a Description, finds nothing new against the report
/// held in the transport, which then fails.
#[tokio::test(start_paused = true)]
async fn an_unsent_report_a_change_was_measured_from_is_looked_at_again() {
    let cov = CovParity::start_local().await;
    cov.link.block_next_send();
    cov.link.fail_next_send();
    let server = &cov.parity.server;
    let link = &cov.link;
    let ai1 = ai();
    let first = server.set_present_value_local(&ai1, PropertyValue::Real(25.0));
    let other = async {
        link.wait_blocked().await;
        let source = LocalCommandSource::ServerDevice;
        let text = PropertyValue::CharacterString("measured".into());
        let other = server
            .write_local(&ai1, P::DESCRIPTION, None, text, None, source)
            .await;
        link.release_sends(1);
        other
    };
    let (first, other) = tokio::join!(first, other);
    first.unwrap();
    other.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let sent: Vec<_> = cov
        .parity
        .sent
        .take()
        .iter()
        .map(|frame| notified(&frame.decode_npdu().payload))
        .collect();
    let value = PropertyValue::Real(25.0);
    assert_eq!(
        sent,
        [value.clone(), value],
        "the failed send, then the look"
    );
    cov.stop().await;
}
