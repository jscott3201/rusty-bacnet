//! The capture pumps: read a source, summarise each frame, and offer them
//! to the UI in batches.
//!
//! Steady capture never waits for the UI. Frames go out when a batch is full or
//! every [`REPORT_EVERY`], whichever is first, so capture shares the UI's
//! event channel with a few events a second. A batch the channel can't
//! take is dropped and its frames counted, beside what the source itself
//! lost (a tap queue or a monitor subscription that filled up), so while a
//! source runs the Capture screen's dropped counter is exact: every frame
//! the source offered is in the ring or in that count. Counts the UI
//! hasn't heard of ride on the next batch that gets through. No lock is
//! held across an await. Final accounting waits for the live UI after the
//! monitor producer stops; shutdown keeps that delivery and input release polled
//! together.

use std::any::Any;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use bacnet_client::client::BACnetClient;
use bacnet_ops::target::AddressStyle;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::mstp::MstpTransport;
use bacnet_transport::mstp::codec::Frame;
use bacnet_transport::mstp::monitor::{LineErrorKind, LineEvent, MonitorFrames, MonitorRecord};
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::{FrameTap, FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bytes::Bytes;
use tokio::sync::mpsc;

use super::redact::scrub;
use super::{Batch, CaptureCommand, CaptureEvent, CaptureFrame, Direction, FrameSummary, Framing};
use crate::tui::message::WorkerEvent;
use crate::tui::monitor::lines::CliSerial;
use crate::tui::worker::{EventSink, Offer};
use bacnet_ops::decode::frame::summarize_frame;

/// What a session's capture is called before its tap is known.
pub(crate) const TAP: &str = "the session's frame tap";

/// Most frames in one batch.
pub(crate) const BATCH: usize = 256;

/// How often counts the UI hasn't heard of are sent on their own.
pub(crate) const REPORT_EVERY: Duration = Duration::from_millis(250);

/// The tap queue: frames the pump may fall behind by.
pub(crate) const TAP_QUEUE: usize = 4_096;

/// Where frames come from.
pub(crate) enum Source {
    /// A transport's frame tap.
    Tap {
        /// The subscription.
        rx: FrameTapReceiver,
        /// The transport's link.
        framing: Framing,
        /// This node's address on it, when it has a UDP one.
        local: Option<SocketAddr>,
        /// This node's address as the device table writes it.
        local_text: String,
    },
    /// The MS/TP monitor's live stream.
    Monitor {
        /// The subscription.
        frames: MonitorFrames,
        /// Wall-clock time of the monitor's start, which records count from.
        epoch: SystemTime,
    },
    /// libpcap on its thread.
    #[cfg(feature = "pcap")]
    Pcap(super::libpcap::Feed),
    /// Frames a test hands in, and its own count of those it lost.
    #[cfg(test)]
    Test {
        /// The frames.
        rx: mpsc::Receiver<CaptureFrame>,
        /// Lost before reaching `rx`.
        lost: std::sync::Arc<std::sync::atomic::AtomicU64>,
    },
}

/// What a source gave. Held only until the frame joins a batch, so its
/// size is not worth a box per frame.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Next {
    /// A frame.
    Frame(CaptureFrame),
    /// The end, and why.
    End(String),
}

impl Source {
    /// The next frame, waiting for one. Cancel safe.
    async fn next(&mut self) -> Next {
        let next = match self {
            Self::Tap {
                rx,
                framing,
                local,
                local_text,
            } => (rx.recv().await).map(|frame| convert(frame, *framing, *local, local_text)),
            Self::Monitor { frames, epoch } => {
                let epoch = *epoch;
                return frames.recv().await.map_or_else(
                    || Next::End("the monitor capture retired".into()),
                    |record| Next::Frame(from_record(record, epoch)),
                );
            }
            #[cfg(feature = "pcap")]
            Self::Pcap(feed) => return feed.recv().await,
            #[cfg(test)]
            Self::Test { rx, .. } => rx.recv().await,
        };
        next.map_or_else(|| Next::End("the source closed".into()), Next::Frame)
    }

    /// The next frame if one is waiting.
    fn try_next(&mut self) -> Option<CaptureFrame> {
        match self {
            Self::Tap {
                rx,
                framing,
                local,
                local_text,
            } => rx
                .try_recv()
                .map(|frame| convert(frame, *framing, *local, local_text)),
            Self::Monitor { frames, epoch } => {
                let epoch = *epoch;
                frames.try_recv().map(|record| from_record(record, epoch))
            }
            #[cfg(feature = "pcap")]
            Self::Pcap(feed) => feed.try_recv(),
            #[cfg(test)]
            Self::Test { rx, .. } => rx.try_recv().ok(),
        }
    }

    /// Frames the source lost so far.
    fn lost(&self) -> u64 {
        match self {
            Self::Tap { rx, .. } => rx.dropped(),
            Self::Monitor { frames, .. } => frames.dropped(),
            #[cfg(feature = "pcap")]
            Self::Pcap(feed) => feed.lost(),
            #[cfg(test)]
            Self::Test { lost, .. } => lost.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// A tapped frame, summarised, with its endpoints.
fn convert(
    frame: TappedFrame,
    framing: Framing,
    local: Option<SocketAddr>,
    local_text: &str,
) -> CaptureFrame {
    let summary = summarize_frame(framing, &frame.bytes);
    let inbound = frame.direction == TapDirection::Inbound;
    let (peer_text, udp) = match frame.peer {
        TapPeer::Udp(peer) => {
            let udp = local.map(|local| {
                if inbound {
                    (peer, local)
                } else {
                    (local, peer)
                }
            });
            (peer.to_string(), udp)
        }
        TapPeer::ScHub(hub) => (format!("hub {}", crate::tui::sc::hub_name(hub)), None),
        TapPeer::Mstp(station) => (AddressStyle::Mstp.format(&[station]), None),
        _ => ("?".to_string(), None),
    };
    let (mut source, mut destination) = if inbound {
        (peer_text, local_text.to_string())
    } else {
        (local_text.to_string(), peer_text)
    };
    // The link's own addresses, where the frame carries them, are the
    // device table's (VMACs, MS/TP stations); B/IP has only UDP's.
    if framing != Framing::Bip {
        source = summary.link_source.clone().unwrap_or(source);
        destination = summary.link_destination.clone().unwrap_or(destination);
    }
    let (source, destination, via) = route(source, destination, &summary, !inbound);
    CaptureFrame {
        time: frame.timestamp,
        framing,
        direction: if inbound {
            Direction::In
        } else {
            Direction::Out
        },
        udp: if framing == Framing::Bip { udp } else { None },
        source,
        destination,
        via,
        bytes: frame.bytes,
        summary,
        redacted: false,
    }
}

/// A routed NPDU's ends as routed stations (`2:0x05`), with the link peer
/// it went through: the router it came from, or the one it was sent to.
pub(crate) fn route(
    source: String,
    destination: String,
    summary: &FrameSummary,
    outbound: bool,
) -> (String, String, Option<String>) {
    if summary.routed_source.is_none() && summary.routed_destination.is_none() {
        return (source, destination, None);
    }
    let via = if outbound {
        destination.clone()
    } else {
        source.clone()
    };
    (
        summary.routed_source.clone().unwrap_or(source),
        summary.routed_destination.clone().unwrap_or(destination),
        Some(via),
    )
}

/// A monitor record as a frame: a valid frame encoded again from its
/// header and data, a line error as a frame with no octets.
fn from_record(record: MonitorRecord, epoch: SystemTime) -> CaptureFrame {
    let time = epoch + record.at;
    let line = |link: &str, problem: String, destination: Option<u8>| {
        let summary = FrameSummary {
            link: link.into(),
            problem: Some(problem),
            link_destination: destination.map(|d| AddressStyle::Mstp.format(&[d])),
            ..FrameSummary::default()
        };
        (Bytes::new(), summary)
    };
    let (bytes, summary) = match record.event {
        LineEvent::Frame { header, .. } => {
            let mut wire = vec![0; 16 + record.data.len() + record.data.len() / 128];
            let frame = Frame::new(
                header.frame_type,
                header.destination,
                header.source,
                &record.data,
            );
            match frame.encode_into(&mut wire) {
                Ok(length) => {
                    wire.truncate(length);
                    let summary = summarize_frame(Framing::Mstp, &wire);
                    (Bytes::from(wire), summary)
                }
                Err(error) => line("frame", format!("not encodable: {error}"), None),
            }
        }
        LineEvent::Error {
            kind,
            destination,
            octets,
        } => line(
            "line-error",
            format!(
                "{}, {octets} octets dropped",
                match kind {
                    LineErrorKind::HeaderCrc => "header-crc",
                    LineErrorKind::DataCrc => "data-crc",
                    _ => "malformed",
                }
            ),
            destination,
        ),
        LineEvent::Noise { octets } => line("noise", format!("{octets} octets of noise"), None),
        LineEvent::FrameAbort => line("frame-abort", "a frame stopped midway".into(), None),
        _ => line("line-event", "unknown line event".into(), None),
    };
    let (source, destination, via) = route(
        summary.link_source.clone().unwrap_or_default(),
        summary.link_destination.clone().unwrap_or_default(),
        &summary,
        false,
    );
    CaptureFrame {
        time,
        framing: Framing::Mstp,
        direction: Direction::Wire,
        udp: None,
        source,
        destination,
        via,
        bytes,
        summary,
        redacted: false,
    }
}

/// The frame tap of a transport the CLI builds, and its link.
pub(crate) fn tap_of<T: TransportPort + 'static>(transport: &T) -> Option<(FrameTap, Framing)> {
    let any = transport as &dyn Any;
    if let Some(bip) = any.downcast_ref::<BipTransport>() {
        return Some((bip.frame_tap().clone(), Framing::Bip));
    }
    if let Some(bip6) = any.downcast_ref::<Bip6Transport>() {
        return Some((bip6.frame_tap().clone(), Framing::Bip6));
    }
    #[cfg(feature = "sc-tls")]
    {
        use bacnet_transport::sc::ScTransport;
        use bacnet_transport::sc_tls::TlsWebSocket;
        if let Some(sc) = any.downcast_ref::<ScTransport<TlsWebSocket>>() {
            return Some((sc.frame_tap().clone(), Framing::Sc));
        }
    }
    let mstp = any.downcast_ref::<MstpTransport<CliSerial>>()?;
    Some((mstp.frame_tap().clone(), Framing::Mstp))
}

/// A session's capture: wait for [`CaptureCommand::Start`], subscribe to
/// the client's frame tap, and pump until the session drops `commands`.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<CaptureCommand>,
    style: AddressStyle,
) {
    while let Some(CaptureCommand::Start) = commands.recv().await {
        let Some((tap, framing)) = tap_of(client.transport()) else {
            let ended = CaptureEvent::Ended {
                what: TAP.into(),
                why: "this transport has no frame tap".into(),
            };
            sink.deliver(WorkerEvent::Capture(ended)).await;
            continue;
        };
        let mac = client.local_mac();
        let local_text = style.format(mac);
        let local = match mac {
            [a, b, c, d, hi, lo] if framing == Framing::Bip => Some(SocketAddr::from((
                [*a, *b, *c, *d],
                u16::from_be_bytes([*hi, *lo]),
            ))),
            _ => None,
        };
        let source = Source::Tap {
            rx: tap.subscribe_with_capacity(TAP_QUEUE),
            framing,
            local,
            local_text: local_text.clone(),
        };
        let started = format!("{} tap at {local_text}", framing.label());
        tracing::info!("capture: {started}");
        if sink
            .deliver(WorkerEvent::Capture(CaptureEvent::Started {
                what: started.clone(),
                session: true,
            }))
            .await
        {
            pump(source, sink, Some(&mut commands), started).await;
        }
        return;
    }
}

/// An owned monitor subscription. The session initiates producer shutdown
/// alongside this pump, so draining to EOF has a finite cutoff and includes
/// unread records and broadcast lag. Final UI delivery cannot delay input stop.
pub(crate) async fn run_monitor(
    sink: &EventSink,
    mut subscriptions: mpsc::Receiver<MonitorFrames>,
    started: tokio::time::Instant,
) {
    if let Some(frames) = subscriptions.recv().await {
        let epoch = SystemTime::now()
            .checked_sub(started.elapsed())
            .unwrap_or_else(SystemTime::now);
        let source = Source::Monitor { frames, epoch };
        let what = "MS/TP monitor, every frame on the line".to_string();
        let started = CaptureEvent::Started {
            what: what.clone(),
            session: true,
        };
        if sink.deliver(WorkerEvent::Capture(started)).await {
            pump(source, sink, None, what).await;
        }
    }
}

/// Start libpcap capture for the whole TUI run, not one session: its
/// frames go to the UI unscoped. The task ends when the UI closes the
/// event channel, which stops the thread.
#[cfg(feature = "pcap")]
pub(crate) fn spawn_pcap(
    device: Option<String>,
    interface: std::net::Ipv4Addr,
    port: u16,
    sink: EventSink,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let what = super::libpcap::describe(&device, port);
        let started = CaptureEvent::Started {
            what: what.clone(),
            session: false,
        };
        if sink.deliver(WorkerEvent::Capture(started)).await {
            let feed = super::libpcap::Feed::start(device, interface, port);
            pump(Source::Pcap(feed), &sink, None, what).await;
        }
    })
}

/// The next command, or never without a channel.
async fn command(commands: &mut Option<&mut mpsc::Receiver<CaptureCommand>>) -> Option<()> {
    match commands {
        Some(rx) => rx.recv().await.map(|_| ()),
        None => std::future::pending().await,
    }
}

/// Pump `source`, which captures `what`, to the UI until the session ends
/// (`commands` closes, or the sink is cancelled or closed) or the source
/// does.
pub(crate) async fn pump(
    mut source: Source,
    sink: &EventSink,
    mut commands: Option<&mut mpsc::Receiver<CaptureCommand>>,
    what: String,
) {
    let mut counts = Counts::default();
    let mut batch = Vec::new();
    let mut report = tokio::time::interval(REPORT_EVERY);
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = sink.cancelled() => return,
            () = sink.closed() => return,
            received = command(&mut commands) => {
                // Another Start is a no-op: this capture already runs.
                if received.is_none() {
                    if matches!(source, Source::Monitor { .. }) {
                        // The owner must stop the monitor concurrently, not
                        // await this pump first. Its closed producer bounds
                        // the drain and lets recv observe unreported lag.
                        commands = None;
                        continue;
                    }
                    counts.finish(batch, &source, sink).await;
                    sink.deliver(WorkerEvent::Capture(CaptureEvent::Ended {
                        what, why: "the monitor or capture session retired".into(),
                    })).await;
                    return;
                }
            }
            next = source.next() => match next {
                // A password leaves no frame before the ring sees it.
                Next::Frame(frame) => {
                    batch.push(scrub(frame));
                    while batch.len() < BATCH {
                        match source.try_next() {
                            Some(frame) => batch.push(scrub(frame)),
                            None => break,
                        }
                    }
                    // A full batch goes now; a partial one waits for the
                    // tick, so a busy source costs the shared channel a
                    // few events a second, not one per wake-up.
                    if batch.len() >= BATCH && !counts.offer(&mut batch, &source, sink) {
                        return;
                    }
                }
                Next::End(reason) => {
                    counts.finish(batch, &source, sink).await;
                    tracing::info!("capture from {what} ended: {reason}");
                    let ended = CaptureEvent::Ended { what, why: reason };
                    sink.deliver(WorkerEvent::Capture(ended)).await;
                    return;
                }
            },
            _ = report.tick() => {
                if !counts.offer(&mut batch, &source, sink) {
                    return;
                }
            }
        }
    }
}

/// What the UI has been told about losses.
#[derive(Default)]
struct Counts {
    /// Losses the UI has counted.
    reported: u64,
    /// Frames in batches the event channel refused.
    refused: u64,
}

impl Counts {
    /// Offer `batch`, with the losses not yet reported; false once the UI
    /// has gone.
    fn offer(&mut self, batch: &mut Vec<CaptureFrame>, source: &Source, sink: &EventSink) -> bool {
        let lost = source.lost() + self.refused;
        if batch.is_empty() && lost == self.reported {
            return true;
        }
        let frames = std::mem::take(batch);
        let count = frames.len() as u64;
        let event = CaptureEvent::Frames(Box::new(Batch {
            frames,
            lost: lost - self.reported,
        }));
        // Refused frames count in the screen's own dropped figure only.
        match sink.offer_uncounted(WorkerEvent::Capture(event)) {
            Offer::Sent => {
                self.reported = lost;
                true
            }
            Offer::Dropped => {
                self.refused += count;
                true
            }
            Offer::Closed => false,
        }
    }

    /// The last batch and counts, when the source has ended: waited for,
    /// so the UI's count is exact to the end.
    async fn finish(&mut self, frames: Vec<CaptureFrame>, source: &Source, sink: &EventSink) {
        let lost = source.lost() + self.refused;
        if frames.is_empty() && lost == self.reported {
            return;
        }
        let batch = Box::new(Batch {
            frames,
            lost: lost - self.reported,
        });
        if sink
            .deliver(WorkerEvent::Capture(CaptureEvent::Frames(batch)))
            .await
        {
            self.reported = lost;
        }
    }
}
