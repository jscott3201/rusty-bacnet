//! The same timed read fixtures cross the simulator's adapter and the actual
//! public SerialPort/MstpTransport boundary. No production source is included.
mod bus_echo;
use std::sync::Mutex as StdMutex;

use bacnet_mstp_codec::{Frame, FrameType};
use bacnet_mstp_core::{Duration as CoreDuration, Instant as CoreInstant};
use bacnet_transport::mstp::{Echo, MstpConfig, MstpTransport, SerialPort};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, Instant};

use crate::line::Character;
use crate::{HostSpec, MasterSpec, Sim, SimConfig};

const GRACE: Duration = Duration::from_millis(5);
const PAYLOAD: &[u8] = &[1, 0, 0xA7];

struct FixtureSerial {
    input: Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    remaining: StdMutex<Vec<u8>>,
    writes: mpsc::UnboundedSender<(Instant, Vec<u8>)>,
    echo: Echo,
    drain_duration: Duration,
}

impl SerialPort for FixtureSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.writes.send((Instant::now(), data.to_vec())).unwrap();
        Ok(())
    }

    async fn drain(&self) -> Result<(), Error> {
        if !self.drain_duration.is_zero() {
            tokio::time::sleep(self.drain_duration).await;
        }
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        {
            let mut remaining = self.remaining.lock().unwrap();
            if !remaining.is_empty() {
                let count = remaining.len().min(buf.len());
                buf[..count].copy_from_slice(&remaining[..count]);
                remaining.drain(..count);
                return Ok(count);
            }
        }
        // recv is cancel safe, and there is no await between taking a chunk
        // and storing its remainder. This also survives the pre-start poll.
        let chunk = self.input.lock().await.recv().await.expect("fixture open");
        let count = chunk.len().min(buf.len());
        buf[..count].copy_from_slice(&chunk[..count]);
        self.remaining
            .lock()
            .unwrap()
            .extend_from_slice(&chunk[count..]);
        Ok(count)
    }

    fn echo(&self) -> Echo {
        self.echo
    }
}

fn encode(kind: FrameType, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    Frame::new(kind, destination, source, data)
        .encode_to_sink(&mut out)
        .unwrap();
    out
}

fn ms(value: u64) -> CoreInstant {
    CoreInstant::from_micros(value * 1000)
}

/// Inject one already-delivered read, independently of the physical line.
/// Wire scenarios test chunk formation separately; these fixtures pin what
/// both adapters must do when SerialPort returns the same timed chunks.
fn model_read(sim: &mut Sim, at: CoreInstant, bytes: &[u8]) {
    sim.run_until(at);
    for &byte in bytes {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(at);
    sim.run_until(at);
}

#[derive(Debug, Clone, Copy)]
enum Case {
    Full,
    Split,
    Missing,
    Mismatch,
    Short,
    Residual,
    Late,
    Expired,
}

impl Case {
    fn reads(self, echo: &[u8], peer: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let with_peer = |mut bytes: Vec<u8>| {
            bytes.extend_from_slice(peer);
            bytes
        };
        match self {
            Self::Full => vec![(1, with_peer(echo.to_vec()))],
            Self::Split => vec![(1, echo[..3].to_vec()), (3, with_peer(echo[3..].to_vec()))],
            Self::Missing => vec![(7, peer.to_vec())],
            Self::Mismatch => {
                let mut bytes = echo.to_vec();
                bytes[2] ^= 0x80;
                vec![(1, with_peer(bytes))]
            }
            Self::Short => vec![(1, with_peer(echo[3..].to_vec()))],
            Self::Residual => vec![(1, echo[..3].to_vec()), (7, with_peer(echo[3..].to_vec()))],
            Self::Late => vec![(7, with_peer(echo.to_vec()))],
            Self::Expired => vec![(12, with_peer(echo.to_vec()))],
        }
    }

    fn counts(self) -> (u64, u64, u64, u64) {
        match self {
            Self::Full | Self::Split | Self::Residual => (0, 0, 0, 0),
            Self::Mismatch | Self::Short => (0, 1, 0, 0),
            Self::Missing | Self::Late => (1, 0, 0, 0),
            // This positive control must leak after the bounded hunt. A
            // blanket source-address discard would wrongly turn it green.
            Self::Expired => (1, 0, 0, 1),
        }
    }
}

async fn settle_task() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

async fn paired(case: Case) {
    let (input, rx) = mpsc::unbounded_channel();
    let (tx, mut writes) = mpsc::unbounded_channel();
    let serial = FixtureSerial {
        input: Mutex::new(rx),
        remaining: StdMutex::new(Vec::new()),
        writes: tx,
        echo: Echo::Full { grace: GRACE },
        drain_duration: Duration::ZERO,
    };
    // One millisecond per octet avoids Tokio's timer granularity changing
    // inclusive filter boundaries. Production accepts this configured baud.
    let mut transport = MstpTransport::new(
        serial,
        MstpConfig {
            this_station: 1,
            max_master: 1,
            baud_rate: 10_000,
            transmit_latency_pad: Duration::ZERO,
            ..MstpConfig::default()
        },
    );
    let mut npdus = transport.start().await.unwrap();
    transport.send_unicast(&[1, 4, 0xC1], &[0]).await.unwrap();
    settle_task().await;
    let token = encode(FrameType::TOKEN, 1, 0, &[]);
    input.send(token.clone()).unwrap();
    let (start, sent) = tokio::time::timeout(Duration::from_millis(100), writes.recv())
        .await
        .unwrap()
        .unwrap();
    let ended = start + Duration::from_millis(sent.len() as u64);

    let mut config = SimConfig::new(10_000);
    config.panic_on_violation = false; // Expired is an intentional leak control.
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(1, 1, 10_000).host(HostSpec {
        echo_grace: Some(CoreDuration::from_millis(5)),
        ..HostSpec::default()
    }));
    sim.enqueue(ms(0), 1, 0, &[1, 4, 0xC1]);
    model_read(&mut sim, ms(0), &token);
    while sim.frames().is_empty() {
        assert!(sim.now() < ms(100));
        sim.run_for(CoreDuration::from_millis(1));
    }
    let frame = sim.frames().first().unwrap().clone();
    assert_eq!(
        sent, frame.octets,
        "{case:?}: both cores chose the same frame"
    );
    let peer = encode(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 0, PAYLOAD);
    for (delay, chunk) in case.reads(&sent, &peer) {
        tokio::time::sleep_until(ended + Duration::from_millis(delay)).await;
        input.send(chunk.clone()).unwrap();
        settle_task().await;
        model_read(
            &mut sim,
            frame.end + CoreDuration::from_millis(delay),
            &chunk,
        );
    }
    settle_task().await;
    let observed = transport.diagnostics().snapshot();
    let actual = (
        observed.echo_missing,
        observed.echo_mismatches,
        observed.echo_fragments_dropped,
        observed.mac.own_address_frames,
    );
    let host = sim.host_stats(1).unwrap();
    let modeled = (
        u64::from(host.echo_missing),
        u64::from(host.echo_mismatches),
        u64::from(host.echo_fragments_dropped),
        u64::from(sim.core(1).unwrap().counters().own_address_frames),
    );
    let mut delivered = Vec::new();
    while let Ok(npdu) = npdus.try_recv() {
        delivered.push(npdu.npdu.to_vec());
    }
    let modeled_payloads: Vec<_> = sim
        .deliveries(1)
        .iter()
        .map(|delivery| delivery.data.clone())
        .collect();
    // Stop and join even when an assertion below fails.
    transport.stop().await.unwrap();
    assert_eq!(actual, case.counts(), "{case:?}: production counters");
    assert_eq!(modeled, actual, "{case:?}: simulator counters");
    assert_eq!(
        delivered,
        vec![PAYLOAD.to_vec()],
        "{case:?}: production peer prefix"
    );
    assert_eq!(
        modeled_payloads, delivered,
        "{case:?}: simulator peer prefix"
    );
    if !matches!(case, Case::Expired) {
        sim.assert_no_violations();
    }
}

/// Both adapters preserve the same peer NPDU and counter outcomes across
/// complete, partial, absent, corrupt and late copies of their own frame.
#[tokio::test(start_paused = true)]
async fn paired_public_transport_echo_fixtures() {
    for case in [
        Case::Full,
        Case::Split,
        Case::Missing,
        Case::Mismatch,
        Case::Short,
        Case::Residual,
        Case::Late,
        Case::Expired,
    ] {
        paired(case).await;
    }
}

/// The public adapter cannot poll an application answer while strip_echo
/// awaits. Its next no-token poll remains anchored to the completed frame,
/// and the finished request's answer is queued without a core callback.
#[tokio::test(start_paused = true)]
async fn paired_ready_answer_during_echo_wait_keeps_wire_deadline() {
    let (input, rx) = mpsc::unbounded_channel();
    let (tx, mut writes) = mpsc::unbounded_channel();
    let mut transport = MstpTransport::new(
        FixtureSerial {
            input: Mutex::new(rx),
            remaining: StdMutex::new(Vec::new()),
            writes: tx,
            echo: Echo::Full {
                grace: Duration::from_millis(40),
            },
            drain_duration: Duration::ZERO,
        },
        MstpConfig {
            this_station: 0,
            max_master: 1,
            baud_rate: 10_000,
            transmit_latency_pad: Duration::ZERO,
            ..MstpConfig::default()
        },
    );
    let mut npdus = transport.start().await.unwrap();
    settle_task().await;
    let received = Instant::now();
    let request = encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 1, &[1, 4, 9]);
    input.send(request.clone()).unwrap();
    let delivery = npdus.recv().await.unwrap();
    let (first_start, first) = writes.recv().await.unwrap();
    let first_end = first_start + Duration::from_millis(first.len() as u64);
    tokio::time::sleep_until(received + Duration::from_millis(240)).await;
    delivery
        .reply_tx
        .unwrap()
        .send(PAYLOAD.to_vec().into())
        .unwrap();
    let (poll_start, poll) = tokio::time::timeout(Duration::from_secs(1), writes.recv())
        .await
        .unwrap()
        .unwrap();
    let queued = transport.diagnostics().snapshot().late_answers_queued;
    transport.stop().await.unwrap();

    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec {
                echo_grace: Some(CoreDuration::from_millis(40)),
                ..HostSpec::default()
            })
            .application(crate::ReplyPolicy::Answer {
                delay: CoreDuration::from_millis(240),
                payload: PAYLOAD.to_vec(),
            }),
    );
    model_read(&mut sim, ms(0), &request);
    sim.run_until(ms(741));
    assert_eq!(first, sim.frames()[0].octets);
    assert_eq!(poll, sim.frames()[1].octets);
    assert_eq!(queued, 1);
    assert_eq!(sim.report().late_answers, 1);
    let actual_silence = poll_start - first_end;
    let modeled_silence = sim.frames()[1].start - sim.frames()[0].end;
    assert_eq!(modeled_silence, CoreDuration::from_millis(500));
    // Tokio's timer wheel can wake one millisecond after a virtual deadline.
    assert!((Duration::from_millis(500)..=Duration::from_millis(501)).contains(&actual_silence));
    sim.assert_no_violations();
}

async fn paired_buffered_token_after_drain(echo: Echo, expected_kind: FrameType) {
    let (input, rx) = mpsc::unbounded_channel();
    let (tx, mut writes) = mpsc::unbounded_channel();
    let mut transport = MstpTransport::new(
        FixtureSerial {
            input: Mutex::new(rx),
            remaining: StdMutex::new(Vec::new()),
            writes: tx,
            echo,
            drain_duration: Duration::from_millis(60),
        },
        MstpConfig {
            this_station: 0,
            max_master: 1,
            baud_rate: 10_000,
            transmit_latency_pad: Duration::ZERO,
            ..MstpConfig::default()
        },
    );
    let mut npdus = transport.start().await.unwrap();
    settle_task().await;
    let received = Instant::now();
    let request = encode(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 1, &[1, 4, 9]);
    input.send(request.clone()).unwrap();
    let delivery = npdus.recv().await.unwrap();
    let (_, postponed) = writes.recv().await.unwrap();
    tokio::time::sleep_until(received + Duration::from_millis(240)).await;
    delivery
        .reply_tx
        .unwrap()
        .send(PAYLOAD.to_vec().into())
        .unwrap();
    let token = encode(FrameType::TOKEN, 0, 1, &[]);
    tokio::time::sleep_until(received + Duration::from_millis(260)).await;
    input.send(token.clone()).unwrap();
    let (_, actual_next) = tokio::time::timeout(Duration::from_millis(500), writes.recv())
        .await
        .unwrap()
        .unwrap();
    transport.stop().await.unwrap();

    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec {
                drain_duration: CoreDuration::from_millis(60),
                echo_grace: match echo {
                    Echo::None => None,
                    Echo::Full { grace } => Some(CoreDuration::from_micros(
                        u64::try_from(grace.as_micros()).unwrap(),
                    )),
                },
                ..HostSpec::default()
            })
            .application(crate::ReplyPolicy::Answer {
                delay: CoreDuration::from_millis(240),
                payload: PAYLOAD.to_vec(),
            }),
    );
    model_read(&mut sim, ms(0), &request);
    model_read(&mut sim, ms(260), &token);
    sim.run_until(ms(320));
    assert_eq!(postponed, encode(FrameType::REPLY_POSTPONED, 1, 0, &[]));
    assert_eq!(postponed, sim.frames()[0].octets);
    let expected_data = if expected_kind == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
        PAYLOAD
    } else {
        &[]
    };
    let expected = encode(expected_kind, 1, 0, expected_data);
    // The very next transmission must have this type, destination and data;
    // eventual delivery after another token would conceal the ordering bug.
    assert_eq!(actual_next, expected, "production: {echo:?}");
    assert_eq!(sim.frames()[1].octets, expected, "simulator: {echo:?}");
    sim.assert_no_violations();
}

#[tokio::test(start_paused = true)]
async fn paired_ready_answer_precedes_buffered_token_after_drain() {
    paired_buffered_token_after_drain(Echo::None, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY).await;
}

/// strip_echo consumes its read before the main loop can poll a ready
/// answer. Moving decisions into host_complete would incorrectly invert it.
#[tokio::test(start_paused = true)]
async fn paired_echo_read_keeps_token_before_ready_answer() {
    paired_buffered_token_after_drain(
        Echo::Full {
            grace: Duration::from_millis(40),
        },
        FrameType::POLL_FOR_MASTER,
    )
    .await;
}
