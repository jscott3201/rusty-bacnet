//! A session killed with SIGKILL after a priority 8 write leaves its ledger
//! entry behind, and the next launch lists it.
//!
//! The parent test runs this test binary again as a child, with only
//! [`killed_child`] selected and the workspace in [`CHILD_ENV`]. The child
//! writes AO:1 at priority 8 on its own in-memory site, prints
//! [`WRITTEN`] and waits to be killed. The wait for that line is bounded,
//! and is the only real-time wait here.

use std::io::{BufRead, BufReader, Write as _};
use std::path::Path;
use std::process::{Command as Process, Stdio};
use std::time::Duration;

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Server, Site};
use super::writes::{LAB, arm, finished, lab, options, write};
use super::*;
use crate::tui::app::Overlay;
use crate::tui::app::safety::Modal;
use crate::tui::profile::Safety;

/// Names the child's workspace; unset, [`killed_child`] does nothing.
const CHILD_ENV: &str = "BACNET_TUI_CRASH_CHILD_WORKSPACE";

/// The child's line once the write is acknowledged.
const WRITTEN: &str = "crash-child: written at priority 8";

/// How long the parent waits for the child's write.
const CHILD_DEADLINE: Duration = Duration::from_secs(120);

/// The child: write AO:1 at priority 8, say so, then wait to be killed.
#[tokio::test(start_paused = true)]
async fn killed_child() {
    let Some(dir) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let dir = Path::new(&dir);
    let mut site = lab(dir, Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    write(&mut site, "42", "8").await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    let mut out = std::io::stdout();
    writeln!(out, "{WRITTEN}").unwrap();
    out.flush().unwrap();
    loop {
        std::thread::park();
    }
}

/// This test's path inside the binary, without the crate name.
fn child_name() -> String {
    let module = module_path!();
    let module = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{module}::killed_child")
}

#[test]
fn a_killed_session_leaves_its_ledger_for_the_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Process::new(std::env::current_exe().unwrap())
        .args([child_name().as_str(), "--exact", "--nocapture"])
        .env(CHILD_ENV, dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) if line.contains(WRITTEN) => {
                    let _ = tx.send(());
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    let written = rx.recv_timeout(CHILD_DEADLINE);
    // SIGKILL on Unix: no destructor, no exit guard, no last save.
    child.kill().unwrap();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    written.expect("the child acknowledged its write before the deadline");
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "killed, not exited");
    }

    // The next launch on the same workspace.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    runtime.block_on(async {
        let options = options(dir.path(), Safety::default(), false);
        let mut site = Site::start_with_options(
            vec![Server::lab(100, LAB)],
            ClientLimits::default(),
            options,
        )
        .await;
        site.pump_until(|app| app.safety.policy.is_some()).await;
        assert_eq!(site.app.safety.leftovers(), 1);
        let entry = &site.app.safety.ledger[0];
        assert_eq!(entry.priority, Some(8));
        assert_eq!(entry.written.text, "42");
        assert!(matches!(
            &site.app.overlay,
            Overlay::Safety(m) if matches!(**m, Modal::Ledger { notice: Some(_), .. })
        ));
        assert_snapshot!("leftover_ledger_120x40", screen(&mut site.app, 120, 40));
        site.stop().await;
    });
}
