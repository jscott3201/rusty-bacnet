use std::future::Future;
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use tokio::sync::Semaphore;
use tokio::time::{advance, Duration, Instant};

use super::*;

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Tx,
    WriteAccepted,
    DrainStarted,
    TxComplete,
    Rx,
    Read,
}

#[derive(Default)]
struct MockDirection {
    events: std::sync::Mutex<Vec<(Event, Instant)>>,
    tx: AtomicBool,
    fail_tx: AtomicBool,
    fail_rx: AtomicBool,
}

impl MockDirection {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push((event, Instant::now()));
    }

    fn assert_events(&self, expected: &[Event]) {
        let events = self.events.lock().unwrap();
        assert_eq!(
            events.iter().map(|(e, _)| e).collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
    }
}

impl DirectionControl for MockDirection {
    fn set_tx_mode(&self) -> Result<(), Error> {
        self.tx.store(true, Ordering::SeqCst);
        self.record(Event::Tx);
        if self.fail_tx.load(Ordering::SeqCst) {
            return Err(Error::Encoding("set TX failed".into()));
        }
        Ok(())
    }

    fn set_rx_mode(&self) -> Result<(), Error> {
        if self.fail_rx.load(Ordering::SeqCst) {
            return Err(Error::Encoding("set RX failed".into()));
        }
        self.tx.store(false, Ordering::SeqCst);
        self.record(Event::Rx);
        Ok(())
    }
}

struct MockSerial {
    direction: Arc<MockDirection>,
    complete: Semaphore,
    fail_write: bool,
    fail_drain: AtomicBool,
}

impl MockSerial {
    fn new(fail_write: bool) -> Self {
        Self {
            direction: Arc::new(MockDirection::default()),
            complete: Semaphore::new(0),
            fail_write,
            fail_drain: AtomicBool::new(false),
        }
    }
}

impl SerialPort for MockSerial {
    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        assert!(self.direction.tx.load(Ordering::SeqCst));
        self.direction.record(Event::WriteAccepted);
        if self.fail_write {
            Err(Error::Encoding("partial write failed".into()))
        } else {
            Ok(())
        }
    }

    async fn drain(&self) -> Result<(), Error> {
        assert!(self.direction.tx.load(Ordering::SeqCst));
        self.direction.record(Event::DrainStarted);
        if self.fail_drain.load(Ordering::SeqCst) {
            return Err(Error::Encoding("drain failed".into()));
        }
        self.complete.acquire().await.unwrap().forget();
        self.direction.record(Event::TxComplete);
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        assert!(!self.direction.tx.load(Ordering::SeqCst));
        self.direction.record(Event::Read);
        buf[0] = 42;
        Ok(1)
    }
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test(start_paused = true)]
async fn gpio_holds_de_until_tx_complete_then_starts_guard_interval() {
    for guard_us in [0, 2_000] {
        let serial = MockSerial::new(false);
        let direction = SoftwareDirection::new(guard_us);
        let mut write = pin!(direction.write(&serial, &*serial.direction, &[1, 2, 3]));
        assert!(poll_once(write.as_mut()).is_pending());
        serial
            .direction
            .assert_events(&[Event::Tx, Event::WriteAccepted, Event::DrainStarted]);

        // Even a long delay after driver acceptance does not establish completion.
        advance(Duration::from_millis(100)).await;
        assert!(poll_once(write.as_mut()).is_pending());
        assert!(serial.direction.tx.load(Ordering::SeqCst));
        serial.complete.add_permits(1);
        let completed_at = Instant::now();
        if guard_us > 0 {
            assert!(poll_once(write.as_mut()).is_pending());
            advance(Duration::from_micros(guard_us - 1)).await;
            assert!(poll_once(write.as_mut()).is_pending());
            assert!(serial.direction.tx.load(Ordering::SeqCst));
            // Tokio timers may round up; lower-bound safety is the invariant.
            advance(Duration::from_millis(2)).await;
        }
        assert!(matches!(poll_once(write.as_mut()), Poll::Ready(Ok(()))));
        serial.direction.assert_events(&[
            Event::Tx,
            Event::WriteAccepted,
            Event::DrainStarted,
            Event::TxComplete,
            Event::Rx,
        ]);
        let events = serial.direction.events.lock().unwrap();
        assert!(events.last().unwrap().1 >= completed_at + Duration::from_micros(guard_us));
    }
}

#[tokio::test]
async fn gpio_partial_write_error_drains_before_restoring_rx() {
    let serial = MockSerial::new(true);
    let direction = SoftwareDirection::new(0);
    let mut write = pin!(direction.write(&serial, &*serial.direction, &[1]));
    assert!(poll_once(write.as_mut()).is_pending());
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    serial.complete.add_permits(1);
    assert!(
        matches!(poll_once(write.as_mut()), Poll::Ready(Err(Error::Encoding(e))) if e == "partial write failed")
    );
    serial.direction.assert_events(&[
        Event::Tx,
        Event::WriteAccepted,
        Event::DrainStarted,
        Event::TxComplete,
        Event::Rx,
    ]);
}

#[tokio::test]
async fn gpio_failed_drain_retains_de_and_read_recovers_only_after_completion() {
    let serial = MockSerial::new(false);
    let direction = SoftwareDirection::new(0);
    serial.fail_drain.store(true, Ordering::SeqCst);
    assert!(
        matches!(direction.write(&serial, &*serial.direction, &[1]).await, Err(Error::Encoding(e)) if e == "drain failed")
    );
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    let mut buf = [0];
    assert!(direction
        .read(&serial, &*serial.direction, &mut buf)
        .await
        .is_err());
    serial.fail_drain.store(false, Ordering::SeqCst);
    let mut read = pin!(direction.read(&serial, &*serial.direction, &mut buf));
    assert!(poll_once(read.as_mut()).is_pending());
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    serial.complete.add_permits(1);
    assert!(matches!(poll_once(read.as_mut()), Poll::Ready(Ok(1))));
    assert!(!serial.direction.tx.load(Ordering::SeqCst));
}

#[tokio::test]
async fn gpio_cancelled_drain_is_recovered_before_reading() {
    let serial = MockSerial::new(false);
    let direction = SoftwareDirection::new(0);
    {
        let mut write = pin!(direction.write(&serial, &*serial.direction, &[1]));
        assert!(poll_once(write.as_mut()).is_pending());
    }
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    serial.complete.add_permits(1);
    let mut buf = [0];
    assert_eq!(
        direction
            .read(&serial, &*serial.direction, &mut buf)
            .await
            .unwrap(),
        1
    );
    serial.direction.assert_events(&[
        Event::Tx,
        Event::WriteAccepted,
        Event::DrainStarted,
        Event::DrainStarted,
        Event::TxComplete,
        Event::Rx,
        Event::Read,
    ]);
}

#[tokio::test]
async fn gpio_serializes_concurrent_writes_through_drain() {
    let serial = MockSerial::new(false);
    let direction = SoftwareDirection::new(0);
    let mut first = pin!(direction.write(&serial, &*serial.direction, &[1]));
    let mut second = pin!(direction.write(&serial, &*serial.direction, &[2]));
    assert!(poll_once(first.as_mut()).is_pending());
    assert!(poll_once(second.as_mut()).is_pending());
    serial
        .direction
        .assert_events(&[Event::Tx, Event::WriteAccepted, Event::DrainStarted]);
    serial.complete.add_permits(1);
    assert!(matches!(poll_once(first.as_mut()), Poll::Ready(Ok(()))));
    assert!(poll_once(second.as_mut()).is_pending());
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    serial.complete.add_permits(1);
    assert!(matches!(poll_once(second.as_mut()), Poll::Ready(Ok(()))));
    serial.direction.assert_events(&[
        Event::Tx,
        Event::WriteAccepted,
        Event::DrainStarted,
        Event::TxComplete,
        Event::Rx,
        Event::Tx,
        Event::WriteAccepted,
        Event::DrainStarted,
        Event::TxComplete,
        Event::Rx,
    ]);
}

#[tokio::test]
async fn gpio_direction_errors_are_reported_and_recoverable() {
    let serial = MockSerial::new(false);
    let direction = SoftwareDirection::new(0);
    serial.direction.fail_tx.store(true, Ordering::SeqCst);
    assert!(
        matches!(direction.write(&serial, &*serial.direction, &[1]).await, Err(Error::Encoding(e)) if e == "set TX failed")
    );
    serial.direction.assert_events(&[Event::Tx, Event::Rx]);

    serial.direction.fail_tx.store(false, Ordering::SeqCst);
    serial.direction.fail_rx.store(true, Ordering::SeqCst);
    serial.complete.add_permits(1);
    assert!(
        matches!(direction.write(&serial, &*serial.direction, &[1]).await, Err(Error::Encoding(e)) if e == "set RX failed")
    );
    assert!(serial.direction.tx.load(Ordering::SeqCst));
    serial.direction.fail_rx.store(false, Ordering::SeqCst);
    serial.complete.add_permits(1);
    direction
        .read(&serial, &*serial.direction, &mut [0])
        .await
        .unwrap();
    assert!(!serial.direction.tx.load(Ordering::SeqCst));
}

#[tokio::test]
async fn custom_backend_without_drain_never_claims_tx_complete() {
    assert!(matches!(crate::mstp::NoSerial.drain().await,
        Err(Error::Transport(e)) if e.kind() == std::io::ErrorKind::Unsupported));
}

#[cfg(unix)]
#[tokio::test]
async fn unix_serial_drain_uses_the_native_backend() {
    let (stream, mut peer) = SerialStream::pair().unwrap();
    let serial = TokioSerialPort {
        inner: Arc::new(Mutex::new(stream)),
    };
    serial.write(&[1, 2, 3]).await.unwrap();
    serial.drain().await.unwrap();
    let mut received = [0; 3];
    peer.read_exact(&mut received).await.unwrap();
    assert_eq!(received, [1, 2, 3]);
    // A pseudo-terminal exercises the syscall path, not physical UART timing.
}

#[cfg(unix)]
#[tokio::test]
async fn native_serial_execution_modes_share_host_timing_harness() {
    use crate::mstp::{port_timing_tests::execution_timing_harness, MstpExecutionMode};

    // Native pseudo-terminals exercise the backend used for UART and USB ports;
    // late-tail delivery models host chunking, not physical RS-485 timing.
    for baud in [9600, 38400, 76800] {
        for chunked in [false, true] {
            for software_direction in [false, true] {
                let mut baseline = None;
                for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
                    let (stream, peer) = SerialStream::pair().unwrap();
                    let serial = TokioSerialPort {
                        inner: Arc::new(Mutex::new(stream)),
                    };
                    let peer = TokioSerialPort {
                        inner: Arc::new(Mutex::new(peer)),
                    };
                    let wire = if software_direction {
                        // Exercise the existing software-direction/drain boundary
                        // with native I/O, without requiring a physical GPIO line.
                        let serial = DirectionTestPort {
                            inner: serial,
                            control: MockDirection::default(),
                            direction: SoftwareDirection::new(0),
                        };
                        execution_timing_harness(serial, peer, Some(mode), baud, chunked).await
                    } else {
                        execution_timing_harness(serial, peer, Some(mode), baud, chunked).await
                    };
                    match &baseline {
                        None => baseline = Some(wire),
                        Some(expected) => assert_eq!(&wire, expected),
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockedIo {
    Read,
    Write,
    Drain,
}

struct GatedSerial {
    inner: crate::mstp::LoopbackSerial,
    operation: BlockedIo,
    entered: tokio::sync::mpsc::UnboundedSender<std::thread::ThreadId>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    blocked_once: AtomicBool,
}

impl GatedSerial {
    fn block(&self, operation: BlockedIo) {
        if operation == self.operation && !self.blocked_once.swap(true, Ordering::SeqCst) {
            self.entered.send(std::thread::current().id()).unwrap();
            // A finite guard makes accidental execution on the app thread fail
            // the test instead of deadlocking the test runner indefinitely.
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
    }
}

impl SerialPort for GatedSerial {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.block(BlockedIo::Read);
        self.inner.read(buf).await
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.block(BlockedIo::Write);
        self.inner.write(data).await
    }

    async fn drain(&self) -> Result<(), Error> {
        self.block(BlockedIo::Drain);
        self.inner.drain().await
    }
}

struct DirectionTestPort<S> {
    inner: S,
    control: MockDirection,
    direction: SoftwareDirection,
}

impl<S: SerialPort> SerialPort for DirectionTestPort<S> {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.direction.read(&self.inner, &self.control, buf).await
    }

    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.direction.write(&self.inner, &self.control, data).await
    }
}

#[tokio::test]
async fn dedicated_blocking_io_keeps_app_responsive_and_stop_waits_for_release() {
    use crate::mstp::{LoopbackSerial, MasterState, MstpConfig, MstpExecutionMode, MstpTransport};
    use crate::mstp_frame::{encode_frame, FrameType, MstpFrame};
    use crate::port::TransportPort;

    for operation in [BlockedIo::Read, BlockedIo::Write, BlockedIo::Drain] {
        let (inner, peer) = LoopbackSerial::pair();
        let (entered, mut events) = tokio::sync::mpsc::unbounded_channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let serial = DirectionTestPort {
            inner: GatedSerial {
                inner,
                operation,
                entered,
                release: std::sync::Mutex::new(blocked),
                blocked_once: AtomicBool::new(false),
            },
            control: MockDirection::default(),
            direction: SoftwareDirection::new(0),
        };
        let mut incoming = bytes::BytesMut::new();
        encode_frame(
            &mut incoming,
            &MstpFrame {
                frame_type: FrameType::PollForMaster,
                destination: 0,
                source: 7,
                data: bytes::Bytes::new(),
            },
        )
        .unwrap();
        peer.write(&incoming).await.unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default())
            .with_execution_mode(MstpExecutionMode::DedicatedThread);
        let _npdus = transport.start().await.unwrap();
        let thread = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(thread, std::thread::current().id());
        transport.send_broadcast(&[1, 0]).await.unwrap();
        let node = transport.node_state().unwrap().clone();
        let stop = transport.stop();
        tokio::pin!(stop);
        tokio::select! {
            _ = &mut stop => panic!("stop completed while backend I/O was blocked"),
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        // The app's current-thread runtime made progress while MAC I/O blocked.
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), stop)
            .await
            .unwrap()
            .unwrap();
        let node = node.lock().await;
        assert!(node.tx_queue.is_empty());
        assert_eq!(node.state, MasterState::Idle);
    }
}
