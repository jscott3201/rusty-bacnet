//! A burst of acknowledgments labelled for another service warns through
//! the client's response throttle (#1950): the first warns, and the rest log
//! at debug and count toward the next warning.

use super::*;
use bacnet_encoding::apdu::{SimpleAck, decode_apdu};
use bacnet_transport::warn_throttle::WARNING_INTERVAL;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

#[tokio::test(start_paused = true)]
async fn a_burst_of_mislabelled_acks_warns_once_per_interval() {
    let Harness {
        client,
        input,
        mut output,
    } = Harness::new(8).await;
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let read = client.read_property(PEER, object, PropertyIdentifier::PRESENT_VALUE, None);
    tokio::pin!(read);
    // Poll the request until it is on the wire, then leave it pending: the
    // dispatch task answers what follows on its own.
    let (sent, _) = tokio::select! {
        _ = &mut read => panic!("the request can't finish before it's answered"),
        sent = output.recv() => sent.unwrap(),
    };
    let Apdu::ConfirmedRequest(sent) = decode_apdu(decode_npdu(sent).unwrap().payload).unwrap()
    else {
        panic!("expected the ReadProperty request");
    };
    const BURST: u64 = 6;
    for _ in 0..BURST {
        let mislabelled = Apdu::SimpleAck(SimpleAck {
            invoke_id: sent.invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        });
        input.send(inbound(mislabelled, None)).await.unwrap();
    }
    // A request the client rejects comes back after everything before it.
    let Apdu::ConfirmedRequest(mut fence) = request(Bytes::new(), true, false) else {
        unreachable!()
    };
    fence.invoke_id = 0xee;
    fence.service_choice = ConfirmedServiceChoice::READ_PROPERTY;
    input
        .send(inbound(Apdu::ConfirmedRequest(fence), None))
        .await
        .unwrap();
    output.recv().await.unwrap();

    let throttle = &client.response_warning;
    assert_eq!(
        throttle.note(),
        None,
        "the first warned; the rest held back"
    );
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        throttle.note(),
        Some(BURST + 1),
        "five held back, the check above, and this one"
    );
    assert_eq!(client.decode_warning.note(), Some(1), "no decode failure");
}
