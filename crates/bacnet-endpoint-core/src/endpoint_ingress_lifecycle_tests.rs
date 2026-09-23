use super::*;
use std::net::SocketAddrV4;
use tokio::sync::Semaphore;

struct GatedStop {
    inner: TestTransport,
    gate: Arc<Semaphore>,
    entered: Arc<Notify>,
    complete: Arc<AtomicUsize>,
    started: bool,
}
impl TransportPort for GatedStop {
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        Some(SocketAddrV4::new(
            [192, 168, 1, 255].into(),
            if self.started { 30001 } else { 0 },
        ))
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.started = true;
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.entered.notify_one();
        self.gate.acquire().await.unwrap().forget();
        self.complete.fetch_add(1, Ordering::SeqCst);
        self.inner.stop().await
    }
    async fn send_unicast(&self, data: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(data, mac).await
    }
    async fn send_broadcast(&self, data: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(data).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}
#[tokio::test]
async fn ingress_canceled_stop_retains_join_and_actual_post_start_bip_fact() {
    let (inner, handle) = test_transport();
    let gate = Arc::new(Semaphore::new(0));
    let entered = Arc::new(Notify::new());
    let complete = Arc::new(AtomicUsize::new(0));
    let mut endpoint = EndpointIngress::new(
        GatedStop {
            inner,
            gate: gate.clone(),
            entered: entered.clone(),
            complete: complete.clone(),
            started: false,
        },
        4,
    );
    assert_eq!(endpoint.bip_broadcast_endpoint().unwrap().port(), 0);
    let receivers = endpoint.start().await.unwrap();
    assert_eq!(receivers.bip_broadcast_endpoint.unwrap().port(), 30001);
    {
        let stop = endpoint.stop();
        tokio::pin!(stop);
        tokio::select! {result=&mut stop=>panic!("stop completed prematurely: {result:?}"), _=entered.notified()=>{}}
    }
    assert_eq!(complete.load(Ordering::SeqCst), 0);
    gate.add_permits(1);
    assert!(matches!(
        timeout(WAIT, endpoint.stop()).await.unwrap().unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(complete.load(Ordering::SeqCst), 1);
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    assert!(endpoint.stop().await.is_err());
    drop(receivers);
    drop(handle);
}
