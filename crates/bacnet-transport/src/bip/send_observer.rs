//! Port-scoped send-attempt observation for integration tests.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

fn observers() -> &'static Mutex<BTreeMap<u16, Arc<AtomicU64>>> {
    static OBSERVERS: OnceLock<Mutex<BTreeMap<u16, Arc<AtomicU64>>>> = OnceLock::new();
    OBSERVERS.get_or_init(Mutex::default)
}
/// Observe logical B/IP socket send requests, including ones that fail, on one
/// local UDP port. Keep the guard until all tasks owning that port have stopped.
/// This watches the native B/IP send seam, not arbitrary process UDP syscalls.
#[cfg_attr(not(feature = "test-support"), allow(dead_code))]
pub struct SendAttemptObserver {
    port: u16,
    attempts: Arc<AtomicU64>,
}
#[cfg_attr(not(feature = "test-support"), allow(dead_code))]
impl SendAttemptObserver {
    /// Watch a nonzero port. Only one observer may own a port at a time.
    pub fn new(port: u16) -> Self {
        assert_ne!(port, 0);
        let attempts = Arc::new(AtomicU64::new(0));
        let mut observers = observers().lock().unwrap();
        assert!(!observers.contains_key(&port), "port already observed");
        observers.insert(port, Arc::clone(&attempts));
        Self { port, attempts }
    }
    /// Number of logical send requests, counted before attempting socket I/O.
    pub fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::Relaxed)
    }
}
impl Drop for SendAttemptObserver {
    fn drop(&mut self) {
        observers().lock().unwrap().remove(&self.port);
    }
}
pub(super) fn attempted(socket: &tokio::net::UdpSocket) {
    if let Ok(address) = socket.local_addr()
        && let Some(counter) = observers().lock().unwrap().get(&address.port())
    {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn counts_failed_socket_attempts_and_in_crate_recorded_sends() {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let observer = SendAttemptObserver::new(socket.local_addr().unwrap().port());
        let socket = crate::bip::socket::BipSocket::new(socket, None);
        // No SO_BROADCAST: a denied send must still be counted.
        assert!(
            socket
                .send_to(&[1], "255.255.255.255:47808".parse().unwrap())
                .await
                .is_err()
        );
        assert_eq!(observer.attempts(), 1);
        drop(socket);
        let raw = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, observer.port))
            .await
            .unwrap();
        let recorded = crate::bip::socket::BipSocket::recording(
            raw,
            crate::bip::send_sink::SendSink::default(),
        );
        recorded
            .send_to(&[1], "127.0.0.1:47808".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(observer.attempts(), 2);
    }
}
