//! The MS/TP passive monitor (#970): a session that listens to a trunk
//! through `bacnet-transport`'s receive-only `MstpMonitor` and never
//! transmits, its analysis, and the hand-off to a participant session.
//!
//! - [`lines`]: where MS/TP sessions open their serial port, and in tests
//!   an in-memory line that holds the port to one owner at a time.
//! - [`worker`]: the monitor session: samples, device instances from I-Am,
//!   lost-token events.
//! - [`analysis`]: station roles, the token ring, Max_Master findings and
//!   the free MAC the join wizard proposes. No I/O.
//!
//! A monitor session is the Passive level of the safety model: no client is
//! built, and the port is opened read only (`ReceiveOnlyPort`). Joining is
//! T2 and goes through the same confirmation as `--mstp` (#969); the
//! coordinator ends the monitor session, which closes the port, before the
//! participant session opens it.
//!
//! The Capture screen (#971) takes its own lossy subscription to the
//! monitor's frame stream: [`worker::serve`] holds the `MstpMonitor`, whose
//! `frames()` gives each subscriber its own stream, and runs the capture
//! pump beside the watcher, which hands it the session's capture commands.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use bacnet_transport::mstp::monitor::{LostToken, MonitorStats, DEFAULT_TIMESTAMP_RESOLUTION};

pub(crate) mod analysis;
pub(crate) mod lines;
pub(crate) mod worker;

pub(crate) use lines::Lines;

/// How often the worker sends a sample to the screen.
pub(crate) const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// Rotation times kept for the sparkline.
pub(crate) const ROTATION_HISTORY: usize = 60;

/// Lost-token events kept for the screen.
pub(crate) const LOST_TOKEN_HISTORY: usize = 16;

/// What a monitor session listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorPlan {
    /// The serial port, as `mstp-ports` lists it.
    pub(crate) port: String,
    /// The line's baud rate.
    pub(crate) baud: u32,
    /// Max_Info_Frames for a station that joins from the monitor.
    pub(crate) max_info_frames: u8,
    /// How uncertain the host's timestamps are (`TimingQuality`).
    pub(crate) resolution: Duration,
}

impl MonitorPlan {
    /// A plan with the default timestamp resolution, a USB adapter's.
    pub(crate) fn new(port: String, baud: u32, max_info_frames: u8) -> Self {
        Self {
            port,
            baud,
            max_info_frames,
            resolution: DEFAULT_TIMESTAMP_RESOLUTION,
        }
    }
}

/// `tui --mstp-monitor PORT`, with `--mstp-baud` and
/// `--mstp-max-info-frames`.
pub(crate) fn plan_from(cli: &crate::args::Cli) -> Option<MonitorPlan> {
    match &cli.command {
        Some(crate::args::Command::Tui {
            mstp_monitor: Some(port),
            ..
        }) => Some(MonitorPlan::new(
            port.clone(),
            cli.mstp_baud,
            cli.mstp_max_info_frames,
        )),
        _ => None,
    }
}

/// The MS/TP part of a session's options.
#[derive(Debug, Clone, Default)]
pub(crate) struct MstpOptions {
    /// What a monitor session (`TransportKind::MstpMonitor`) listens to.
    pub(crate) monitor: Option<MonitorPlan>,
    /// Where serial ports are opened.
    pub(crate) lines: Lines,
    /// How a joined station's thread is scheduled (`--mstp-rt-priority`,
    /// `--mstp-cpus`), whichever way it joins.
    pub(crate) thread: bacnet_transport::mstp::MstpThreadConfig,
}

/// One reading of the monitor, for the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorSample {
    /// Time since the monitor started.
    pub(crate) elapsed: Duration,
    /// The statistics engine's state.
    pub(crate) stats: MonitorStats,
    /// Device instances from I-Am, by MAC.
    pub(crate) devices: BTreeMap<u8, u32>,
    /// MACs seen sending a network-layer message or a routed NPDU.
    pub(crate) routers: BTreeSet<u8>,
    /// The latest ring rotation times, oldest first, as the lowest master
    /// sees them: the mean of those between two looks.
    pub(crate) rotations: VecDeque<Duration>,
    /// The latest lost-token events, oldest first.
    pub(crate) lost: VecDeque<LostToken>,
    /// Live records this worker lost by falling behind.
    pub(crate) dropped: u64,
}

/// What the monitor worker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MonitorReport {
    /// Listening, nothing transmitted.
    Started {
        /// The port.
        port: String,
        /// Its baud rate.
        baud: u32,
    },
    /// A new reading.
    Sample(Box<MonitorSample>),
    /// The monitor ended by itself: the port closed or failed.
    Ended(String),
}
