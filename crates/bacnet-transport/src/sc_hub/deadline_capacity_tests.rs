use super::deadline_test_support::*;
use super::heartbeat_test_support::clients;
use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn hub_admission_abort_before_first_poll_reclaims_slot() {
    let tls = TestTls::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _peer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (tcp, address) = listener.accept().await.unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let admission = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
    assert_eq!(active.load(Ordering::Acquire), 1);
    let task = tokio::spawn(super::connection::serve_connection(
        tcp,
        address,
        tls.acceptor,
        ([0x10; 6], [0x10; 16]),
        clients(),
        ScHubHandshakeTimeouts::default(),
        admission,
    ));
    // Current-thread runtime: no await occurs between spawn and abort.
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        active.load(Ordering::Acquire),
        0,
        "unpolled task stranded its admitted slot"
    );
}

#[tokio::test]
async fn hub_admission_abort_during_tls_reclaims_slot() {
    let tls = TestTls::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _peer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (tcp, address) = listener.accept().await.unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let admission = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
    let mut operation = Box::pin(super::connection::serve_connection(
        tcp,
        address,
        tls.acceptor,
        ([0x10; 6], [0x10; 16]),
        clients(),
        ScHubHandshakeTimeouts::default(),
        admission,
    ));
    assert!(futures_util::poll!(&mut operation).is_pending()); // actual TLS wait has started
    assert_eq!(active.load(Ordering::Acquire), 1);
    let task = tokio::spawn(operation);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(active.load(Ordering::Acquire), 0);
}

pub(super) struct CountedHub {
    pub address: SocketAddr,
    pub active: Arc<AtomicUsize>,
    pub clients: Clients,
    task: JoinHandle<()>,
}

impl CountedHub {
    pub async fn start(tls: &TestTls, timeouts: ScHubHandshakeTimeouts) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let clients = clients();
        let task = tokio::spawn(super::connection::accept_loop_with_counter(
            listener,
            tls.acceptor.clone(),
            [0x10; 6],
            [0x10; 16],
            clients.clone(),
            timeouts,
            active.clone(),
        ));
        Self {
            address,
            active,
            clients,
            task,
        }
    }
}

impl Drop for CountedHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn hub_actual_512_stalled_slots_expire_and_legitimate_mtls_recovers() {
    let tls = TestTls::new();
    let timeouts = ScHubHandshakeTimeouts::new(
        Duration::from_secs(300),
        Duration::from_secs(10),
        Duration::from_secs(5),
    )
    .unwrap();
    let hub = CountedHub::start(&tls, timeouts).await;
    let mut stalled = Vec::new();
    for admitted in 1..=512 {
        let mut tcp = TcpStream::connect(hub.address).await.unwrap();
        // TCP connect completion is not admission. Observe the production counter.
        until(|| hub.active.load(Ordering::Acquire) == admitted).await;
        if admitted % 2 == 0 {
            tcp.write_all(&[0x16, 0x03, 0x03, 0, 16, 1]).await.unwrap();
        }
        stalled.push(tcp);
    }
    let mut rejected = TcpStream::connect(hub.address).await.unwrap();
    let mut byte = [0; 1];
    assert!(matches!(
        poll_io(rejected.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    assert_eq!(hub.active.load(Ordering::Acquire), 512);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    tokio::time::resume();
    drop(stalled);

    let mut ws = tls.websocket(hub.address).await;
    ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    assert!(
        matches!(poll_io(ws.next()).await, Some(Ok(Message::Binary(data))) if data[0..4] == [7, 0, 0x22, 0x33])
    );
    assert!(hub.clients.lock().await.contains_key(&[0x42; 6]));
    assert_eq!(
        hub.active.load(Ordering::Acquire),
        1,
        "established clients must retain their active slot"
    );
    ws.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    assert!(hub.clients.lock().await.is_empty());
}

#[tokio::test]
async fn hub_phase_error_upgrade_timeout_and_connect_timeout_release_slots() {
    let tls = TestTls::new();
    let timeouts = ScHubHandshakeTimeouts::new(
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_secs(5),
    )
    .unwrap();
    let hub = CountedHub::start(&tls, timeouts).await;
    let mut bad_tls = TcpStream::connect(hub.address).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    bad_tls.write_all(b"not a TLS record").await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;

    let mut bad_http = tls
        .connect_tls(TcpStream::connect(hub.address).await.unwrap())
        .await;
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    bad_http.write_all(b"invalid HTTP\r\n\r\n").await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;

    let _idle_http = tls
        .connect_tls(TcpStream::connect(hub.address).await.unwrap())
        .await;
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    until(|| hub.active.load(Ordering::Acquire) == 0).await;

    let mut ws = tls.websocket(hub.address).await;
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_millis(5002)).await;
    assert!(matches!(
        poll_io(ws.next()).await,
        Some(Ok(Message::Close(_)))
    ));
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    assert!(hub.clients.lock().await.is_empty());
}
