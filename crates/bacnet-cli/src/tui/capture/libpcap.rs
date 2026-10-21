//! Wire capture with libpcap (`tui --capture-device`): B/IP datagrams on
//! the `-p` port, read on a blocking thread that feeds a bounded channel.
//!
//! The one-shot `capture` stops on Ctrl-C through a `ctrlc` handler. In the
//! TUI Ctrl-C is a key, so the thread stops instead when the [`Feed`] is
//! dropped: it checks a flag between reads, which time out every
//! [`READ_TIMEOUT_MS`] milliseconds.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use tokio::sync::mpsc;

use super::worker::{route, Next};
use super::{CaptureFrame, Direction, Framing};
use crate::commands::capture::{build_filter, resolve_device, udp_datagram};
use crate::decode::frame::summarize_frame;

/// How long one read waits before the thread looks at its stop flag.
pub(crate) const READ_TIMEOUT_MS: i32 = 100;

/// Datagrams the thread may get ahead of the pump by.
const QUEUE: usize = 4_096;

/// The receiving end of the capture thread. Dropping it stops the thread.
pub(crate) struct Feed {
    rx: mpsc::Receiver<Result<CaptureFrame, String>>,
    lost: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Feed {
    /// Open `device` (or the one with `interface`, or the default) and
    /// capture UDP on `port` on a new thread.
    pub(crate) fn start(device: Option<String>, interface: Ipv4Addr, port: u16) -> Self {
        let (tx, rx) = mpsc::channel(QUEUE);
        let lost = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_lost, thread_stop) = (lost.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("bacnet-pcap".into())
            .spawn(move || read(device, interface, port, &tx, &thread_lost, &thread_stop));
        let feed = Self { rx, lost, stop };
        if let Err(error) = spawned {
            tracing::error!("capture thread: {error}");
        }
        feed
    }

    /// The next frame, or the end with its reason.
    pub(crate) async fn recv(&mut self) -> Next {
        match self.rx.recv().await {
            Some(Ok(frame)) => Next::Frame(frame),
            Some(Err(error)) => Next::End(error),
            None => Next::End("the capture thread ended".into()),
        }
    }

    /// A frame if one is waiting; an error waits for [`recv`](Self::recv).
    pub(crate) fn try_recv(&mut self) -> Option<CaptureFrame> {
        match self.rx.try_recv() {
            Ok(Ok(frame)) => Some(frame),
            Ok(Err(error)) => {
                tracing::warn!("libpcap: {error}");
                None
            }
            Err(_) => None,
        }
    }

    /// Datagrams dropped because the queue was full.
    pub(crate) fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }
}

/// The thread: open, filter, then read until stopped or failed.
fn read(
    device: Option<String>,
    interface: Ipv4Addr,
    port: u16,
    tx: &mpsc::Sender<Result<CaptureFrame, String>>,
    lost: &AtomicU64,
    stop: &AtomicBool,
) {
    let opened = resolve_device(&device, interface).and_then(|device| {
        let name = device.name.clone();
        let mut capture = pcap::Capture::from_device(device)
            .and_then(|c| {
                c.snaplen(65_535)
                    .timeout(READ_TIMEOUT_MS)
                    .immediate_mode(true)
                    .open()
            })
            .map_err(|e| format!("{name}: {e}"))?;
        capture
            .filter(&build_filter(port, &None), true)
            .map_err(|e| format!("{name}: {e}"))?;
        Ok(capture)
    });
    let mut capture = match opened {
        Ok(capture) => capture,
        Err(error) => {
            let _ = tx.try_send(Err(error));
            return;
        }
    };
    let datalink = capture.get_datalink();
    while !stop.load(Ordering::Relaxed) {
        let packet = match capture.next_packet() {
            Ok(packet) => packet,
            Err(pcap::Error::TimeoutExpired) => continue,
            Err(error) => {
                let _ = tx.try_send(Err(error.to_string()));
                return;
            }
        };
        let Some((source, destination, payload)) = udp_datagram(packet.data, datalink) else {
            continue;
        };
        let since = Duration::new(
            u64::try_from(packet.header.ts.tv_sec).unwrap_or_default(),
            u32::try_from(packet.header.ts.tv_usec).unwrap_or_default() * 1_000,
        );
        let summary = summarize_frame(Framing::Bip, payload);
        let (shown_source, shown_destination, via) =
            route(source.to_string(), destination.to_string(), &summary, false);
        let frame = CaptureFrame {
            time: UNIX_EPOCH + since,
            framing: Framing::Bip,
            direction: Direction::Wire,
            udp: Some((SocketAddr::V4(source), SocketAddr::V4(destination))),
            source: shown_source,
            destination: shown_destination,
            via,
            bytes: Bytes::copy_from_slice(payload),
            summary,
        };
        match tx.try_send(Ok(frame)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                lost.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
}

/// What this capture listens to, for the Capture screen's title.
pub(crate) fn describe(device: &Option<String>, port: u16) -> String {
    let device = device.as_deref().unwrap_or("the default device");
    format!("libpcap on {device}, UDP port {port}")
}
