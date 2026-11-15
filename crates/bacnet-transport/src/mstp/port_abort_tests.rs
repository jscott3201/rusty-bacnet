//! The MS/TP transport's abort, drop and stop: what each leaves of the
//! node's task, its thread and the serial port.

use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;

struct ExclusiveSerial(Arc<AtomicBool>);

impl ExclusiveSerial {
    fn open(in_use: Arc<AtomicBool>) -> Option<Self> {
        in_use
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(in_use))
    }
}

impl Drop for ExclusiveSerial {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl SerialPort for ExclusiveSerial {
    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
        pending().await
    }
}

async fn reopen(in_use: &Arc<AtomicBool>) -> ExclusiveSerial {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(serial) = ExclusiveSerial::open(in_use.clone()) {
                break serial;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted receive task retained the serial resource")
}

#[tokio::test]
async fn abort_takes_task_and_releases_serial_for_reopen() {
    let in_use = Arc::new(AtomicBool::new(false));
    let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
    let mut transport = MstpTransport::new(serial, MstpConfig::default());
    let _rx = transport.start().await.unwrap();
    assert!(ExclusiveSerial::open(in_use.clone()).is_none());

    transport.abort();
    assert!(lock(&transport.running).task.is_none());
    assert!(transport.link.is_none());
    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
    drop(reopen(&in_use).await);
    assert!(!in_use.load(Ordering::SeqCst));
}

/// A stop cancelled while it waits leaves the rest to the next stop,
/// which returns only once the node's thread has let go of the serial
/// port (#2617). Each round cancels the first stop after one more poll,
/// so it is cancelled in the task's join and in the thread's end.
#[tokio::test]
async fn a_cancelled_stop_leaves_the_rest_to_the_next() {
    for polls in 1..=3 {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default())
            .with_execution_mode(MstpExecutionMode::dedicated_thread());
        let _rx = transport.start().await.unwrap();
        let finished = {
            let mut stop = Box::pin(transport.stop());
            let mut finished = false;
            for _ in 0..polls {
                let polled =
                    std::future::poll_fn(|cx| std::task::Poll::Ready(stop.as_mut().poll(cx))).await;
                if polled.is_ready() {
                    finished = true;
                    break;
                }
                tokio::task::yield_now().await;
            }
            finished
        };
        if !finished {
            // The thread stays with the transport until a stop has seen
            // it end.
            assert!(lock(&transport.running).runtime.is_some(), "{polls} polls");
        }
        transport.stop().await.unwrap();
        assert!(
            !in_use.load(Ordering::SeqCst),
            "the second stop joined the thread ({polls} polls)"
        );
        assert!(lock(&transport.running).runtime.is_none());
        assert!(lock(&transport.running).task.is_none());
    }
}

/// On the caller's runtime, where no thread's end hides a lost task, a stop
/// cancelled in the task's join puts the task back: the next stop joins it,
/// and returns only once the task has let go of the serial port (#2617).
#[tokio::test]
async fn a_stop_cancelled_in_the_join_leaves_the_task_to_the_next() {
    let in_use = Arc::new(AtomicBool::new(false));
    let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
    let mut transport = MstpTransport::new(serial, MstpConfig::default());
    let _rx = transport.start().await.unwrap();
    {
        let mut stop = Box::pin(transport.stop());
        let polled =
            std::future::poll_fn(|cx| std::task::Poll::Ready(stop.as_mut().poll(cx))).await;
        assert!(polled.is_pending(), "the first stop waits in the join");
    }
    assert!(
        lock(&transport.running).task.is_some(),
        "the cancelled stop put the task back"
    );
    transport.stop().await.unwrap();
    assert!(
        !in_use.load(Ordering::SeqCst),
        "the second stop joined the task"
    );
    assert!(lock(&transport.running).task.is_none());
}

#[tokio::test]
async fn drop_aborts_task_and_releases_serial_for_reopen() {
    let in_use = Arc::new(AtomicBool::new(false));
    let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
    let mut transport = MstpTransport::new(serial, MstpConfig::default());
    let _rx = transport.start().await.unwrap();

    drop(transport);
    drop(reopen(&in_use).await);
    assert!(!in_use.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_refused_configuration_keeps_the_serial_port() {
    let in_use = Arc::new(AtomicBool::new(false));
    let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
    let config = MstpConfig {
        this_station: 9,
        max_master: 7,
        ..MstpConfig::default()
    };
    let mut transport = MstpTransport::new(serial, config);
    let error = transport.start().await.unwrap_err();
    assert!(error.to_string().contains("exceeds configured max_master"));
    assert!(transport.serial.is_some(), "the serial port was consumed");
}
