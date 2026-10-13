//! An in-memory duplex serial line with a scripted peer, for tests that
//! drive the transport frame by frame in paused Tokio time.
//!
//! The station under test writes into [`WireSerial`], which records each
//! write with the moment it started; the test plays the other stations by
//! pushing encoded frames into the station's receive side. The station times
//! each frame by the baud rate (the line itself delivers at once), so a
//! frame's end is its start plus its octets times the octet time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_mstp_codec::{decode_frame_in_place, Frame, FrameType};
use bacnet_types::error::Error;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{Duration, Instant};

use super::{Echo, MstpConfig, MstpTransport, SerialPort};
use crate::port::{ReceivedNpdu, TransportPort};

/// How the station's port reports transmit completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrainMode {
    /// `drain` returns at once, as for an in-memory port.
    Immediate,
    /// `drain` is unsupported, as for a custom backend that has none.
    Unsupported,
    /// `drain` fails with a transport error other than unsupported, as a
    /// port whose transmit completion is unknown does.
    Fails,
}

/// The station's end of the line.
pub(super) struct WireSerial {
    rx: Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    leftover: StdMutex<Vec<u8>>,
    /// The station's own receive side, for its echo.
    own: mpsc::UnboundedSender<Vec<u8>>,
    writes: mpsc::UnboundedSender<(Instant, Vec<u8>)>,
    port: Port,
    echoing: Arc<AtomicBool>,
    fail_writes: Arc<AtomicBool>,
}

/// How the station's port behaves.
#[derive(Debug, Clone, Copy)]
pub(super) struct Port {
    /// The echo the port declares to the transport.
    pub(super) echo: Echo,
    /// Whether each write comes back to the port's receiver. A port can
    /// declare an echo it does not produce, or leave a test to script it.
    pub(super) echoes: bool,
    pub(super) drain: DrainMode,
}

impl Port {
    /// A port without echo whose drain returns at once.
    pub(super) const PLAIN: Self = Self {
        echo: Echo::None,
        echoes: false,
        drain: DrainMode::Immediate,
    };
}

impl SerialPort for WireSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(Error::Transport(std::io::Error::other(
                "simulated write failure",
            )));
        }
        let _ = self.writes.send((Instant::now(), data.to_vec()));
        if self.echoing.load(Ordering::SeqCst) {
            let _ = self.own.send(data.to_vec());
        }
        Ok(())
    }

    async fn drain(&self) -> Result<(), Error> {
        match self.port.drain {
            DrainMode::Immediate => Ok(()),
            DrainMode::Unsupported => Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no drain",
            ))),
            DrainMode::Fails => Err(Error::Transport(std::io::Error::other(
                "simulated drain failure",
            ))),
        }
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        {
            let mut leftover = self.leftover.lock().unwrap();
            if !leftover.is_empty() {
                let count = leftover.len().min(buf.len());
                buf[..count].copy_from_slice(&leftover[..count]);
                leftover.drain(..count);
                return Ok(count);
            }
        }
        let mut rx = self.rx.lock().await;
        // An empty chunk carries nothing; a read that returned nothing would
        // be the end of the line.
        let chunk = loop {
            match rx.recv().await {
                Some(chunk) if chunk.is_empty() => {}
                Some(chunk) => break chunk,
                None => return Err(Error::Transport(std::io::Error::other("line closed"))),
            }
        };
        drop(rx);
        let count = chunk.len().min(buf.len());
        buf[..count].copy_from_slice(&chunk[..count]);
        self.leftover
            .lock()
            .unwrap()
            .extend_from_slice(&chunk[count..]);
        Ok(count)
    }

    fn echo(&self) -> Echo {
        self.port.echo
    }
}

/// A frame the station wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sent {
    /// When its write started.
    pub(super) start: Instant,
    /// When its last octet left, by the baud rate.
    pub(super) end: Instant,
    pub(super) frame_type: FrameType,
    pub(super) destination: u8,
    pub(super) source: u8,
    pub(super) data: Vec<u8>,
}

/// Encode a frame for the wire.
pub(super) fn encode(frame_type: FrameType, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = Vec::new();
    Frame::new(frame_type, destination, source, data)
        .encode_to_sink(&mut wire)
        .expect("a valid test frame");
    wire
}

/// The station under test and the line around it.
pub(super) struct Bus {
    pub(super) transport: MstpTransport<WireSerial>,
    pub(super) npdus: mpsc::Receiver<ReceivedNpdu>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    writes: mpsc::UnboundedReceiver<(Instant, Vec<u8>)>,
    pub(super) fail_writes: Arc<AtomicBool>,
    /// Whether each write comes back to the station, as `Port::echoes`
    /// started it; a test can turn it off to script the echo itself.
    pub(super) echoing: Arc<AtomicBool>,
    /// One octet's time on the wire.
    pub(super) octet: Duration,
    /// T_turnaround: 40 bit times.
    pub(super) turnaround: Duration,
    pub(super) station: u8,
}

impl Bus {
    /// Start a station with `config` on a line without echo.
    pub(super) async fn start(config: MstpConfig) -> Self {
        Self::start_with(config, Port::PLAIN).await
    }

    /// Start a station with `config` on `port`.
    pub(super) async fn start_with(config: MstpConfig, port: Port) -> Self {
        let (input, rx) = mpsc::unbounded_channel();
        let (write_tx, writes) = mpsc::unbounded_channel();
        let fail_writes = Arc::new(AtomicBool::new(false));
        let echoing = Arc::new(AtomicBool::new(port.echoes));
        let baud = u64::from(config.baud_rate);
        let station = config.this_station;
        let serial = WireSerial {
            rx: Mutex::new(rx),
            leftover: StdMutex::new(Vec::new()),
            own: input.clone(),
            writes: write_tx,
            port,
            echoing: echoing.clone(),
            fail_writes: fail_writes.clone(),
        };
        let mut transport = MstpTransport::new(serial, config);
        let npdus = transport.start().await.expect("the station starts");
        Self {
            transport,
            npdus,
            input,
            writes,
            fail_writes,
            echoing,
            octet: Duration::from_micros((10_000_000u64).div_ceil(baud)),
            turnaround: Duration::from_micros((40_000_000u64).div_ceil(baud)),
            station,
        }
    }

    /// The peer puts raw octets on the line now, in one chunk.
    pub(super) fn send_raw(&self, octets: &[u8]) -> Instant {
        self.input.send(octets.to_vec()).expect("the station reads");
        Instant::now()
    }

    /// The peer sends a frame now. Returns when its last octet was read
    /// (the line delivers at once).
    pub(super) fn send(
        &self,
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data: &[u8],
    ) -> Instant {
        self.send_raw(&encode(frame_type, destination, source, data))
    }

    /// The next frame the station writes, however long it takes.
    pub(super) async fn next(&mut self) -> Sent {
        let (start, octets) = self.writes.recv().await.expect("the station runs");
        // In place on a copy, so an extended frame decodes too.
        let mut copy = octets.clone();
        let (frame, consumed) =
            decode_frame_in_place(&mut copy).expect("the station writes valid frames");
        assert_eq!(consumed, octets.len(), "one frame per write");
        let count = u32::try_from(octets.len()).unwrap();
        Sent {
            start,
            end: start + self.octet * count,
            frame_type: frame.header.frame_type,
            destination: frame.header.destination,
            source: frame.header.source,
            data: frame.data.to_vec(),
        }
    }

    /// Sleep until `at`.
    pub(super) async fn until(&self, at: Instant) {
        tokio::time::sleep_until(at).await;
    }

    /// Bring the station into a ring whose only other master is `peer`:
    /// give it the token, let it poll up to `peer`, answer as `peer`, and
    /// take the token it passes. Returns that Token.
    pub(super) async fn join(&mut self, peer: u8) -> Sent {
        self.send(FrameType::TOKEN, self.station, peer, &[]);
        loop {
            let poll = self.next().await;
            assert_eq!(poll.frame_type, FrameType::POLL_FOR_MASTER, "{poll:?}");
            if poll.destination == peer {
                self.until(poll.end + self.turnaround).await;
                self.send(FrameType::REPLY_TO_POLL_FOR_MASTER, self.station, peer, &[]);
                break;
            }
        }
        let token = self.next().await;
        assert_eq!(
            (token.frame_type, token.destination),
            (FrameType::TOKEN, peer),
            "{token:?}"
        );
        token
    }

    /// As `peer`, take the token the station passed and pass it back after
    /// T_turnaround, which shows the station its successor took it.
    pub(super) async fn return_token(&mut self, token: &Sent, peer: u8) -> Instant {
        self.until(token.end + self.turnaround).await;
        self.send(FrameType::TOKEN, self.station, peer, &[])
    }

    /// Stop the station.
    pub(super) async fn stop(mut self) {
        self.transport.stop().await.expect("the station stops");
    }
}

/// A station at 9600 baud with addresses `station` and `max_master`, the
/// host's timeouts, and no transmit-latency pad on any platform, so a port
/// without a drain is timed by the wire alone unless a test sets one.
pub(super) fn config(station: u8, max_master: u8) -> MstpConfig {
    MstpConfig {
        this_station: station,
        max_master,
        max_info_frames: 1,
        baud_rate: 9600,
        extended_frames: false,
        transmit_latency_pad: Duration::ZERO,
        ..MstpConfig::default()
    }
}

/// T_usage_timeout as [`config`] sets it: the host's 35 ms.
pub(super) const T_USAGE_TIMEOUT: Duration = super::HOST_USAGE_TIMEOUT;

/// T_reply_timeout as [`config`] sets it: the host's 300 ms.
pub(super) const T_REPLY_TIMEOUT: Duration = super::HOST_REPLY_TIMEOUT;
