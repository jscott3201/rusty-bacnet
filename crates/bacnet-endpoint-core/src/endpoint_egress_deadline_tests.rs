use super::*;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration, Instant};

struct GateTransport {
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::Sender<Vec<u8>>,
    gate: Arc<Semaphore>,
}
impl TransportPort for GateTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.receiver.take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent.send(mac.to_vec()).await.unwrap();
        self.gate.acquire().await.unwrap().forget();
        Ok(())
    }
    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        unreachable!()
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}
#[tokio::test]
async fn endpoint_egress_discards_expired_and_canceled_commands_before_transport() {
    let (_input, receiver) = mpsc::channel(8);
    let (sent, mut emissions) = mpsc::channel(8);
    let gate = Arc::new(Semaphore::new(0));
    let mut ingress = EndpointIngress::new(
        GateTransport {
            receiver: Some(receiver),
            sent,
            gate: Arc::clone(&gate),
        },
        8,
    );
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
                NetworkPriority::NORMAL,
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
