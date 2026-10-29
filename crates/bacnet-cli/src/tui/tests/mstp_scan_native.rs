//! Native Linux descriptor/configuration observer for the actual scanner.
//! A PTY qualifies access/release orchestration, never physical baud detection.
use super::*;
use crate::tui::monitor::scan::{RowStatus, ScanPhase, Suggestion};
use crate::tui::monitor::{Lines, MonitorPlan, MstpOptions};
use crate::tui::worker::session::{ConnectPlan, SessionOptions, TransportKind};
use crate::tui::worker::{EventSink, coordinator};
use std::collections::BTreeSet;
use std::io::Write;
use std::os::fd::AsRawFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_serial::{SerialPort as _, SerialStream};

const ORIGINAL: u32 = 19200;
fn marker(phase: &str, path: &str, fd: i32) {
    std::io::stderr()
        .lock()
        .write_all(
            format!("RB_SCAN_TRACE phase={phase} path={path} fd={fd} original={ORIGINAL}\n")
                .as_bytes(),
        )
        .unwrap();
}
async fn event(
    app: &mut App,
    events: &mut mpsc::Receiver<WorkerEvent>,
    commands: &mpsc::Sender<Command>,
) {
    let next = events
        .recv()
        .await
        .expect("native scanner coordinator ended");
    for command in worker(app, next) {
        commands.send(command).await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn mstp_scan_system_lines_native_observer() {
    // Keep virtual time under explicit test control while native reader threads
    // deliver their acknowledgments. This task owns no input and is test-only.
    let clock_hold = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let (mut peer, setup) = SerialStream::pair().unwrap();
    let path = setup.name().unwrap();
    let setup_fd = setup.as_raw_fd();
    marker("setup", &path, setup_fd);
    // Independent controls precede the measured production lifetimes.
    {
        let mut writable = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        marker("control", &path, writable.as_raw_fd());
        let bytes = b"native-scan-observer-control";
        writable.write_all(bytes).unwrap();
        let mut received = vec![0; bytes.len()];
        peer.read_exact(&mut received).await.unwrap();
        assert_eq!(received, bytes);
    }
    {
        let mut readonly = std::fs::File::open(&path).unwrap();
        marker("probe", &path, readonly.as_raw_fd());
        assert_eq!(
            readonly.write(b"denied-probe").unwrap_err().raw_os_error(),
            Some(9)
        );
    }
    marker("measure_begin", &path, setup_fd);
    let monitor = MonitorPlan::new(path.clone(), ORIGINAL, 1);
    let options = SessionOptions {
        mstp: MstpOptions {
            monitor: Some(monitor.clone()),
            lines: Lines::System,
            ..Default::default()
        },
        ..Default::default()
    };
    let plan = ConnectPlan {
        kind: TransportKind::MstpMonitor,
        args: crate::tui::profile::Profile::default()
            .transport_args()
            .unwrap(),
        await_interface: false,
    };
    let (sink, mut events) = EventSink::channel(64);
    let (commands, rx) = mpsc::channel(16);
    let task = coordinator::spawn(Some(plan), None, sink, rx, options);
    let mut app = App::new(
        AppConfig {
            transport: "MSTP",
            style: AddressStyle::Mstp,
            color: false,
            picker: None,
            join: None,
            passive: true,
            now: t0(),
        },
        LogRing::new(10),
    );
    app.start_monitor(&monitor, false);
    while !app.monitor.as_ref().unwrap().live {
        event(&mut app, &mut events, &commands).await;
        assert!(!app.quit);
    }
    let mut script = bacnet_transport::mstp::monitor::script::LineScript::new(ORIGINAL);
    script.frame(bacnet_transport::mstp::codec::FrameType::TOKEN, 2, 1, &[]);
    let bytes: Vec<u8> = script
        .chunks()
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    AsyncWriteExt::write_all(&mut peer, &bytes).await.unwrap();
    while app
        .monitor
        .as_ref()
        .unwrap()
        .latest
        .as_ref()
        .is_none_or(|sample| sample.stats.frames.total() == 0)
    {
        tokio::time::advance(Duration::from_secs(1)).await;
        event(&mut app, &mut events, &commands).await;
    }
    assert!(press(&mut app, KeyCode::Char(':')).is_empty());
    type_text(&mut app, "mstp scan 1");
    for command in press(&mut app, KeyCode::Enter) {
        commands.send(command).await.unwrap();
    }
    let mut fed = BTreeSet::new();
    let mut advanced = BTreeSet::new();
    let mut restored_fed = false;
    loop {
        event(&mut app, &mut events, &commands).await;
        assert!(!app.quit, "{:?}", app.exit_error);
        let state = app.monitor.as_ref().unwrap();
        let scan = state.scan.as_ref().unwrap();
        assert!(
            !matches!(scan.phase, ScanPhase::Stopped(_)),
            "{:?}",
            scan.phase
        );
        if scan.phase == ScanPhase::Restored && state.live {
            assert!(
                scan.rows
                    .iter()
                    .all(|row| row.status == RowStatus::Complete && row.evidence.valid > 0)
            );
            assert_eq!(
                scan.suggestion(),
                Suggestion::Ambiguous(crate::mstp::BAUD_RATES.to_vec())
            );
            if !restored_fed {
                AsyncWriteExt::write_all(&mut peer, &bytes).await.unwrap();
                restored_fed = true;
            }
            if state
                .latest
                .as_ref()
                .is_some_and(|sample| sample.stats.frames.total() > 0)
            {
                break;
            }
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        for row in &scan.rows {
            if row.status == RowStatus::Measuring {
                if fed.insert(row.baud) {
                    AsyncWriteExt::write_all(&mut peer, &bytes).await.unwrap();
                }
                if row.evidence.valid > 0 && advanced.insert(row.baud) {
                    tokio::time::advance(Duration::from_secs(1)).await;
                }
            }
        }
    }
    assert_eq!(
        advanced.into_iter().collect::<Vec<_>>(),
        crate::mstp::BAUD_RATES
    );
    marker("restored", &path, setup_fd);
    drop(commands);
    task.await.unwrap();
    marker("measure_end", &path, setup_fd);
    // Setup owner survives every gap but is excluded from measured production
    // opens. No ordinary serial write capability entered the monitor session.
    drop(setup);
    drop(peer);
    clock_hold.abort();
    assert!(clock_hold.await.unwrap_err().is_cancelled());
}
