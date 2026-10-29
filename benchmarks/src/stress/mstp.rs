//! MS/TP confirmed-request throughput and latency against the wire limit.
//!
//! Two master stations, a client at MAC 0 and a server at MAC 1, run the
//! real MS/TP transport over [`SimWire`]: a [`LoopbackSerial`] pair that
//! delivers each frame's octets no sooner than a UART shifting them out at
//! the baud rate would, in 4-millisecond chunks, and counts what it carries.
//! One client reads a property in a loop at each baud rate; the scenario
//! reports the rate and latency it got next to the wire limit: the
//! rate a line at that baud rate carries the same data frames at, with only
//! the token pass and the mandatory turnarounds between them.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bacnet_client::client::BACnetClient;
use bacnet_server::server::BACnetServer;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport, SerialPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_transport::port::TransportPort;

use bacnet_objects::device::DeviceConfig;

use crate::stress::harness::{make_large_db_with, stress_device_config};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

/// Bits one octet takes on the line: a start bit, eight data bits, a stop bit.
pub const BITS_PER_OCTET: u64 = 10;
/// The turnaround before a station transmits after receiving: 40 bit times.
pub const TURNAROUND_BITS: u64 = 40;
/// The octets of a Token frame: the 8-octet header and no data.
pub const TOKEN_OCTETS: u64 = 8;

/// The confirmed requests per second a line at `baud` can carry when the
/// request and reply data frames of each come to `data_octets` octets
/// between them and the token goes round the two stations once per request:
/// the request, the reply and two Token frames, each after a turnaround.
pub fn wire_limit(baud: u32, data_octets: f64) -> f64 {
    let token_bits = 2.0 * (TOKEN_OCTETS * BITS_PER_OCTET) as f64;
    let bits = data_octets * BITS_PER_OCTET as f64 + token_bits + 4.0 * TURNAROUND_BITS as f64;
    f64::from(baud) / bits
}

/// A frame's time on the line at `baud`.
pub fn frame_time(octets: usize, baud: u32) -> Duration {
    Duration::from_nanos(octets as u64 * BITS_PER_OCTET * 1_000_000_000 / u64::from(baud))
}

/// What a [`SimWire`] pair carried, both directions together.
#[derive(Debug, Default)]
pub struct WireCounters {
    /// Every octet written.
    pub octets: AtomicU64,
    /// Every frame (one write each).
    pub frames: AtomicU64,
    /// Octets of BACnet data frames: Types 5 and 6, and the extended 32 and 33.
    pub data_octets: AtomicU64,
    /// BACnet data frames.
    pub data_frames: AtomicU64,
}

/// A snapshot of [`WireCounters`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WireTotals {
    /// Every octet written.
    pub octets: u64,
    /// Every frame.
    pub frames: u64,
    /// Octets of BACnet data frames.
    pub data_octets: u64,
    /// BACnet data frames.
    pub data_frames: u64,
}

impl WireCounters {
    /// The totals so far.
    pub fn totals(&self) -> WireTotals {
        WireTotals {
            octets: self.octets.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            data_octets: self.data_octets.load(Ordering::Relaxed),
            data_frames: self.data_frames.load(Ordering::Relaxed),
        }
    }
}

impl std::ops::Sub for WireTotals {
    type Output = Self;
    fn sub(self, earlier: Self) -> Self {
        Self {
            octets: self.octets - earlier.octets,
            frames: self.frames - earlier.frames,
            data_octets: self.data_octets - earlier.data_octets,
            data_frames: self.data_frames - earlier.data_frames,
        }
    }
}

/// One end of a simulated RS-485 line: a [`LoopbackSerial`] whose writes
/// take the frame's time on the line, the peer hearing each chunk as its
/// last octet would arrive.
pub struct SimWire {
    inner: LoopbackSerial,
    baud: u32,
    counters: Arc<WireCounters>,
}

impl SimWire {
    /// Two connected ends at `baud`, and the counters they share.
    pub fn pair(baud: u32) -> (Self, Self, Arc<WireCounters>) {
        let (a, b) = LoopbackSerial::pair();
        let counters = Arc::new(WireCounters::default());
        let end = |inner| Self {
            inner,
            baud,
            counters: counters.clone(),
        };
        (end(a), end(b), counters)
    }
}

/// Wait until `end`: sleep to a millisecond before it (Tokio's timer
/// granularity), then yield until it passes, so a short frame is not
/// stretched to a whole millisecond.
async fn wait_until(end: Instant) {
    let early = end.checked_sub(Duration::from_millis(1));
    if let Some(early) = early.filter(|early| *early > Instant::now()) {
        tokio::time::sleep_until(early.into()).await;
    }
    while Instant::now() < end {
        tokio::task::yield_now().await;
    }
}

/// How much of a frame the line delivers at a time: what it carries in
/// about 4 milliseconds, as a UART's receive interrupts or a USB adapter's
/// packets would hand it over. Well inside T_frame_abort, and early enough
/// that a long reply begins inside the requester's reply timeout.
const CHUNK_TIME: Duration = Duration::from_millis(4);

impl SerialPort for SimWire {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        // Each chunk reaches the peer once its last octet has left; only
        // the frame's end is timed to the microsecond.
        let start = Instant::now();
        let chunk = (CHUNK_TIME.as_nanos() as usize * self.baud as usize
            / (BITS_PER_OCTET as usize * 1_000_000_000))
            .max(1);
        let mut sent = 0;
        while sent < data.len() {
            let end = (sent + chunk).min(data.len());
            let due = start + frame_time(end, self.baud);
            if end == data.len() {
                wait_until(due).await;
            } else {
                tokio::time::sleep_until(due.into()).await;
            }
            self.inner.write(&data[sent..end]).await?;
            sent = end;
        }
        let octets = data.len() as u64;
        self.counters.octets.fetch_add(octets, Ordering::Relaxed);
        self.counters.frames.fetch_add(1, Ordering::Relaxed);
        // Octet 2 is the frame type, after the two-octet preamble.
        if matches!(data.get(2), Some(5 | 6 | 32 | 33)) {
            self.counters
                .data_octets
                .fetch_add(octets, Ordering::Relaxed);
            self.counters.data_frames.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn drain(&self) -> Result<(), Error> {
        // The write returned when its last bit left.
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }
}

/// A master station's configuration: masters 0 and 1 only.
pub fn station(this_station: u8, baud_rate: u32, extended_frames: bool) -> MstpConfig {
    MstpConfig {
        this_station,
        max_master: 1,
        baud_rate,
        extended_frames,
        ..MstpConfig::default()
    }
}

/// What the client reads, and so how long the reply is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// An Analog Input's Present_Value: a 17-octet reply APDU.
    Small,
    /// The Device's Object_List of 92 objects, about 470 octets: near the
    /// 480 a standard frame carries.
    Standard,
    /// The Device's Object_List of 289 objects, about 1,460 octets, over
    /// extended frames: near the 1,476-octet APDU limit.
    Extended,
}

impl Reply {
    /// Parse `small`, `standard` or `extended`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "small" => Some(Self::Small),
            "standard" => Some(Self::Standard),
            "extended" => Some(Self::Extended),
            _ => None,
        }
    }

    /// The Analog Inputs the server's database holds.
    fn analog_inputs(self) -> u32 {
        match self {
            Self::Small => 10,
            Self::Standard => 89,
            Self::Extended => 286,
        }
    }

    /// The object and property the client reads.
    fn target(self) -> (ObjectIdentifier, PropertyIdentifier) {
        match self {
            Self::Small => (
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                PropertyIdentifier::PRESENT_VALUE,
            ),
            Self::Standard | Self::Extended => (
                ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
                PropertyIdentifier::OBJECT_LIST,
            ),
        }
    }
}

/// Read until one read succeeds (the token ring forms first), for up to 20 seconds.
async fn warm_up<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> bool {
    let give_up = Instant::now() + Duration::from_secs(20);
    while Instant::now() < give_up {
        if client.read_property(mac, oid, property, None).await.is_ok() {
            return true;
        }
    }
    false
}

/// Run `duration_secs` of back-to-back reads at each baud rate. Returns the
/// curve (the parameter is the baud rate) and, per rate, the wire figures.
pub async fn run(
    reply: Reply,
    duration_secs: u64,
    bauds: &[u64],
) -> (Vec<DegradationPoint>, Vec<serde_json::Value>) {
    let mut curve = Vec::new();
    let mut wire = Vec::new();
    let extended = reply == Reply::Extended;
    let (oid, property) = reply.target();

    for &baud in bauds {
        let baud = u32::try_from(baud).expect("baud rate fits in u32");
        eprintln!("--- {baud} baud, {reply:?} reply ---");
        let (client_end, server_end, counters) = SimWire::pair(baud);
        let server_port = MstpTransport::new(server_end, station(1, baud, extended));
        let identity = stress_device_config(1234)
            .identity
            .resolve(server_port.local_receive_apdu_capacity())
            .unwrap();
        let mut server = BACnetServer::<MstpTransport<SimWire>>::generic_builder()
            .device(identity.clone())
            .transport(server_port)
            .database(make_large_db_with(
                DeviceConfig {
                    identity,
                    ..stress_device_config(1234)
                },
                reply.analog_inputs(),
            ))
            .build()
            .await
            .unwrap();
        let mut client = BACnetClient::<MstpTransport<SimWire>>::generic_builder()
            .transport(MstpTransport::new(client_end, station(0, baud, extended)))
            .apdu_timeout_ms(10_000)
            .build()
            .await
            .unwrap();

        if !warm_up(&client, &[1], oid, property).await {
            eprintln!("  no reply within 20 seconds; skipped");
            let _ = client.stop().await;
            let _ = server.stop().await;
            continue;
        }

        let mut recorder = LatencyRecorder::new();
        let before = counters.totals();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(duration_secs);
        while Instant::now() < deadline {
            let start = Instant::now();
            match client.read_property(&[1], oid, property, None).await {
                Ok(_) => recorder.record_success(start),
                Err(_) => recorder.record_failure(),
            }
        }
        let measured = started.elapsed();
        let elapsed = measured.as_secs_f64();
        let carried = counters.totals() - before;
        let _ = client.stop().await;
        let _ = server.stop().await;

        let ok = recorder.successful().max(1);
        let throughput = recorder.successful() as f64 / elapsed;
        let data_octets = carried.data_octets as f64 / ok as f64;
        let limit = wire_limit(baud, data_octets);
        let busy = (carried.octets * BITS_PER_OCTET) as f64 / f64::from(baud) / elapsed;
        let stats = recorder.stats();
        eprintln!(
            "  ok={} err={} throughput={throughput:.1}/s p50={}us p99={}us \
             data_octets_per_request={data_octets:.0} frames_per_request={:.2} \
             wire_limit={limit:.1}/s efficiency={:.0}% line_busy={:.0}%",
            recorder.successful(),
            recorder.failed(),
            stats.p50,
            stats.p99,
            carried.frames as f64 / ok as f64,
            100.0 * throughput / limit,
            100.0 * busy,
        );
        wire.push(serde_json::json!({
            "baud": baud,
            "data_octets_per_request": data_octets,
            "frames_per_request": carried.frames as f64 / ok as f64,
            "wire_limit_per_sec": limit,
            "efficiency_pct": 100.0 * throughput / limit,
            "line_busy_pct": 100.0 * busy,
        }));
        curve.push(DegradationPoint::from_recorder(
            u64::from(baud),
            &recorder,
            measured,
        ));
    }
    (curve, wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 23-octet request and a 29-octet reply at 9600 baud: 520 bits of
    /// data, 160 of Token frames and 160 of turnaround, so 9600 / 840.
    #[test]
    fn wire_limit_counts_data_tokens_and_turnarounds() {
        let limit = wire_limit(9600, 52.0);
        assert!((limit - 9600.0 / 840.0).abs() < 1e-9, "{limit}");
    }

    #[test]
    fn frame_time_is_ten_bits_per_octet() {
        assert_eq!(frame_time(96, 9600), Duration::from_millis(100));
        assert_eq!(frame_time(0, 9600), Duration::ZERO);
    }

    /// A write reaches the peer no sooner than its time on the line, and
    /// only data frames count as data.
    #[tokio::test]
    async fn sim_wire_holds_a_write_for_its_time_on_the_line() {
        let (a, b, counters) = SimWire::pair(9600);
        let token = [0x55, 0xFF, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        let mut data = [0u8; 48];
        data[..3].copy_from_slice(&[0x55, 0xFF, 0x06]);
        let start = Instant::now();
        a.write(&token).await.unwrap();
        a.write(&data).await.unwrap();
        assert!(start.elapsed() >= frame_time(56, 9600));
        // Delivered in chunks of 3 octets (4 milliseconds at 9600 baud).
        let mut heard = Vec::new();
        let mut buf = [0u8; 64];
        while heard.len() < 56 {
            let read = b.read(&mut buf).await.unwrap();
            assert!(read <= 3, "{read}");
            heard.extend_from_slice(&buf[..read]);
        }
        assert_eq!(heard[..8], token);
        assert_eq!(heard[8..], data);
        let totals = counters.totals();
        assert_eq!(
            totals,
            WireTotals {
                octets: 56,
                frames: 2,
                data_octets: 48,
                data_frames: 1,
            }
        );
    }
}
