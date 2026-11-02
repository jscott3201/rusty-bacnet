//! A dedicated-thread node on the test's paused time (#2284): a stalled
//! test thread moves none of its deadlines, and they come once the test's
//! runtime goes idle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bacnet_mstp_codec::{FrameType, decode_frame};
use tokio::time::Instant;

use super::test_support::config;
use super::{
    LoopbackSerial, MasterState, MstpExecutionMode, MstpThreadConfig, MstpTransport, SerialPort,
};
use crate::port::TransportPort;

/// T_no_token, after which a silent line's node starts to poll.
const T_NO_TOKEN: Duration = Duration::from_millis(500);

/// The test's runtime, as the node's time.
fn on_test_time() -> MstpExecutionMode {
    MstpExecutionMode::DedicatedThread(
        MstpThreadConfig::new().with_test_time(tokio::runtime::Handle::current()),
    )
}

/// What the peer has been sent already, without waiting for more.
async fn sent_now(peer: &LoopbackSerial) -> Option<Vec<u8>> {
    let mut buf = [0u8; 64];
    tokio::select! {
        biased;
        read = peer.read(&mut buf) => Some(buf[..read.unwrap()].to_vec()),
        () = std::future::ready(()) => None,
    }
}

/// The #2269 stall, which on the wall clock lets T_no_token pass and the
/// node poll: here the node's thread runs on while the test's is blocked,
/// but its time stands still with the test runtime's. Once the test waits,
/// that runtime runs the node's deadlines in order, and the node polls for
/// a master T_no_token after it started, on paused time.
#[tokio::test(start_paused = true)]
async fn a_stalled_test_thread_moves_no_dedicated_deadline() {
    let (serial, peer) = LoopbackSerial::pair();
    let mut transport =
        MstpTransport::new(serial, config(3, 7)).with_execution_mode(on_test_time());
    let _rx = transport.start().await.unwrap();
    let started = Instant::now();
    assert!(transport.execution_status().unwrap().dedicated_thread);

    // A real stall of the test's thread, longer than T_no_token.
    std::thread::sleep(T_NO_TOKEN + Duration::from_millis(200));
    assert_eq!(Instant::now(), started, "the test's time stood still");
    assert_eq!(sent_now(&peer).await, None, "the node sent nothing");
    let state = transport.diagnostics().node_state().map(|node| node.state);
    assert_eq!(state, Some(MasterState::Idle));

    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(Duration::from_secs(5), peer.read(&mut buf))
        .await
        .expect("the node polls once the test's runtime is idle")
        .unwrap();
    let (frame, _) = decode_frame(&buf[..read]).unwrap();
    assert_eq!(frame.header.frame_type, FrameType::POLL_FOR_MASTER);
    let waited = Instant::now() - started;
    assert!(
        waited >= T_NO_TOKEN && waited < Duration::from_secs(1),
        "polled at {waited:?} of paused time"
    );
    transport.stop().await.unwrap();
}

/// A port whose first write blocks the node's thread for a while, as a
/// starved node thread would sit.
struct StallingSerial {
    inner: LoopbackSerial,
    stall: AtomicBool,
}

impl SerialPort for StallingSerial {
    async fn write(&self, data: &[u8]) -> Result<(), bacnet_types::error::Error> {
        if self.stall.swap(false, Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(300));
        }
        self.inner.write(data).await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, bacnet_types::error::Error> {
        self.inner.read(buf).await
    }
}

/// The other way round: the node's thread is busy (here, blocked in a
/// write) while the test's runtime has nothing to run. That runtime must
/// not advance its clock to the test's next timer meanwhile, or a test
/// waiting on the node would time out early.
#[tokio::test(start_paused = true)]
async fn a_busy_dedicated_node_holds_the_test_clock() {
    let (serial, peer) = LoopbackSerial::pair();
    let serial = StallingSerial {
        inner: serial,
        stall: AtomicBool::new(true),
    };
    let mut transport =
        MstpTransport::new(serial, config(3, 7)).with_execution_mode(on_test_time());
    let _rx = transport.start().await.unwrap();
    let started = Instant::now();
    // The first poll goes out about 30 ms after T_no_token; the wait ends
    // 100 ms after it, well inside the write's real 300 ms.
    let mut buf = [0u8; 64];
    let read = tokio::time::timeout(T_NO_TOKEN + Duration::from_millis(100), peer.read(&mut buf))
        .await
        .expect("the test's clock waits for the busy node")
        .unwrap();
    let (frame, _) = decode_frame(&buf[..read]).unwrap();
    assert_eq!(frame.header.frame_type, FrameType::POLL_FOR_MASTER);
    assert!(Instant::now() - started >= T_NO_TOKEN);
    transport.stop().await.unwrap();
}

/// Stopped, the node holds the test's runtime no longer: paused time
/// advances again, and the runtime shuts down when the test ends.
#[tokio::test(start_paused = true)]
async fn a_stopped_dedicated_node_releases_the_test_clock() {
    let (serial, _peer) = LoopbackSerial::pair();
    let mut transport =
        MstpTransport::new(serial, config(3, 7)).with_execution_mode(on_test_time());
    let _rx = transport.start().await.unwrap();
    transport.stop().await.unwrap();
    let before = Instant::now();
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    assert_eq!(Instant::now() - before, Duration::from_secs(3_600));
}

#[test]
fn test_time_keeps_the_config_comparable() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let config = MstpThreadConfig::new().with_test_time(runtime.handle().clone());
    assert!(config.has_test_time());
    assert_eq!(config, config.clone());
    assert_ne!(
        config,
        MstpThreadConfig::new().with_test_time(runtime.handle().clone())
    );
    assert_ne!(config, MstpThreadConfig::new());
    assert!(!MstpThreadConfig::new().has_test_time());
}
