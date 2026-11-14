use super::*;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use tokio::sync::Semaphore;
use tokio::time::{Duration, Instant, timeout};

/// A link fed by `receiver`; each send (either kind) reports to `sent` (the
/// destination MAC of a unicast, the NPDU of a broadcast), then waits for a
/// permit from `gate`.
fn gate_transport(
    receiver: mpsc::Receiver<ReceivedNpdu>,
    sent: mpsc::Sender<Vec<u8>>,
    gate: Arc<Semaphore>,
) -> TestTransport {
    TestTransport::builder()
        .inbound(receiver)
        .on_send(move |frame| {
            let (sent, gate) = (sent.clone(), Arc::clone(&gate));
            async move {
                let report = if frame.broadcast {
                    frame.npdu.to_vec()
                } else {
                    frame.mac.to_vec()
                };
                sent.send(report).await.unwrap();
                gate.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .build()
}
#[tokio::test]
async fn endpoint_egress_discards_expired_and_canceled_commands_before_transport() {
    let (_input, receiver) = mpsc::channel(8);
    let (sent, mut emissions) = mpsc::channel(8);
    let gate = Arc::new(Semaphore::new(0));
    let mut ingress = EndpointIngress::new(gate_transport(receiver, sent, Arc::clone(&gate)), 8);
    let receivers = ingress.start().await.unwrap();
    let enqueue = |mac, deadline| {
        receivers
            .egress
            .admit_apdu(
                vec![0x10, 8],
                EndpointApduDestination::Direct {
                    destination_mac: MacAddr::from_slice(&[mac]),
                },
                false,
                NetworkPriority::Normal,
                Vec::new(),
                deadline,
            )
            .unwrap()
    };
    let first = enqueue(2, None);
    assert_eq!(emissions.recv().await.unwrap(), vec![2]);
    let expired = enqueue(3, Some(Instant::now() + Duration::from_millis(20)));
    let canceled = enqueue(4, Some(Instant::now() + Duration::from_secs(10)));
    drop(canceled);
    let final_send = enqueue(5, None);
    tokio::time::sleep(Duration::from_millis(30)).await;
    gate.add_permits(2);
    assert!(first.complete().await.result.is_ok());
    let outcome = expired.complete().await;
    assert!(!outcome.attempted);
    assert!(outcome.result.is_err());
    assert!(final_send.complete().await.result.is_ok());
    assert_eq!(emissions.recv().await.unwrap(), vec![5]);
    assert!(emissions.try_recv().is_err());
    // A deadline also bounds a transport future already in progress.
    let in_flight = enqueue(6, Some(Instant::now() + Duration::from_millis(20)));
    assert_eq!(emissions.recv().await.unwrap(), vec![6]);
    let outcome = timeout(Duration::from_secs(1), in_flight.complete())
        .await
        .unwrap();
    assert!(outcome.attempted);
    assert!(outcome.result.is_err());
    ingress.stop().await.unwrap();
}

#[tokio::test]
async fn network_number_egress_cancellation_retracts_queued_and_in_progress_commands() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let (_input, receiver) = mpsc::channel(8);
    let (sent, mut emissions) = mpsc::channel(8);
    let gate = Arc::new(Semaphore::new(0));
    let mut ingress = EndpointIngress::new(gate_transport(receiver, sent, gate.clone()), 8);
    let receivers = ingress.start().await.unwrap();
    let enqueue = |mac| {
        receivers
            .egress
            .admit_apdu(
                vec![0x10, 8],
                EndpointApduDestination::Direct {
                    destination_mac: MacAddr::from_slice(&[mac]),
                },
                false,
                NetworkPriority::Normal,
                vec![],
                None,
            )
            .unwrap()
    };
    let first = enqueue(2);
    assert_eq!(emissions.recv().await.unwrap(), vec![2]);
    let npdu = vec![1, 0x80, 0x13, 0, 17, 1];
    let mut control = Box::pin(receivers.egress.send_network_number_is(npdu.clone()));
    poll_fn(|cx| {
        assert!(control.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(control); // closes completion while the command is queued behind first
    let last = enqueue(3);
    gate.add_permits(2);
    assert!(first.complete().await.result.is_ok());
    assert!(last.complete().await.result.is_ok());
    assert_eq!(emissions.recv().await.unwrap(), vec![3]);
    assert!(emissions.try_recv().is_err());
    let mut control = Box::pin(receivers.egress.send_network_number_is(npdu.clone()));
    tokio::select! { biased; result = &mut control => panic!("send unexpectedly completed: {result:?}"), sent = emissions.recv() => assert_eq!(sent.unwrap(), npdu) }
    drop(control); // in-progress transport future is now canceled
    let last = enqueue(4);
    gate.add_permits(1);
    assert!(
        timeout(Duration::from_secs(1), last.complete())
            .await
            .unwrap()
            .result
            .is_ok()
    );
    assert_eq!(emissions.recv().await.unwrap(), vec![4]);
    assert!(emissions.try_recv().is_err());
    assert!(
        receivers
            .egress
            .send_network_number_is(vec![1, 0, 0x10, 8])
            .await
            .is_err()
    );
    ingress.stop().await.unwrap();
    assert!(receivers.egress.send_network_number_is(npdu).await.is_err());
}

#[tokio::test]
async fn owned_response_command_cancellation_is_independent_of_deadline() {
    use bacnet_network::response_route::ResponseRoute;
    let (_input, receiver) = mpsc::channel(4);
    let (sent, mut emissions) = mpsc::channel(4);
    let gate = Arc::new(Semaphore::new(0));
    let mut ingress = EndpointIngress::new(gate_transport(receiver, sent, gate.clone()), 1);
    let receivers = ingress.start().await.unwrap();
    let enqueue = |mac| {
        receivers.egress.admit_response_apdu(
            vec![0x60, 1, 9],
            MacAddr::from_slice(&[mac]),
            None,
            ResponseRoute::unverified(),
        )
    };
    let first = enqueue(2).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), emissions.recv())
            .await
            .unwrap()
            .unwrap(),
        [2]
    );
    let canceled = enqueue(3).unwrap();
    assert!(matches!(
        enqueue(4),
        Err(EndpointEgressAdmissionError::QueueFull)
    ));
    drop(canceled);
    gate.add_permits(1);
    assert!(first.complete().await.result.is_ok());
    receivers.egress.wait_for_capacity().await.unwrap();
    let last = enqueue(5).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), emissions.recv())
            .await
            .unwrap()
            .unwrap(),
        [5]
    );
    gate.add_permits(1);
    assert!(last.complete().await.result.is_ok());
    ingress.stop().await.unwrap();
    assert!(matches!(
        enqueue(6),
        Err(EndpointEgressAdmissionError::Closed)
    ));
    assert!(emissions.try_recv().is_err());
}
