//! Plain `bacnet` on a real pseudo-terminal (#974): it opens the TUI, and
//! `--no-tui` or `BACNET_NO_TUI` keep it out, with the usage and exit
//! status 2. Unix only: the pair comes from `openpty`.
#![cfg(all(unix, feature = "tui"))]
#![forbid(unsafe_code)]

use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What the TUI writes first: the alternate screen.
const ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
/// How long a child may take to show what the test waits for.
const DEADLINE: Duration = Duration::from_secs(20);

/// A pseudo-terminal pair, 120 by 40: (master, slave).
fn openpty() -> (OwnedFd, OwnedFd) {
    let size = nix::pty::Winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pair = nix::pty::openpty(&size, None).expect("openpty");
    (pair.master, pair.slave)
}

/// A `bacnet` child on the slave side of a fresh pseudo-terminal, and what
/// it writes there, chunk by chunk.
struct OnPty {
    child: Child,
    output: mpsc::Receiver<Vec<u8>>,
    seen: Vec<u8>,
    _config: tempfile::TempDir,
}

impl OnPty {
    fn spawn(args: &[&str], no_tui_env: Option<&str>) -> Self {
        let (master, slave) = openpty();
        let config = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bacnet"));
        cmd.args(args)
            .env("TERM", "xterm-256color")
            // An empty profile store: the TUI opens on profile creation and
            // sends nothing.
            .env("BACNET_CONFIG_DIR", config.path())
            .env_remove("BACNET_NO_TUI")
            .env_remove("RUST_LOG")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        if let Some(value) = no_tui_env {
            cmd.env("BACNET_NO_TUI", value);
        }
        let child = cmd.spawn().unwrap();
        // The parent's copies of the slave go with `cmd`, so the master
        // reads end of file once the child is gone.
        drop(cmd);
        let (sender, output) = mpsc::channel();
        let mut master = File::from(master);
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            // Linux reports EIO, not end of file, once the slave is closed.
            while let Ok(read @ 1..) = master.read(&mut buffer) {
                if sender.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            output,
            seen: Vec::new(),
            _config: config,
        }
    }

    /// Read until `needle` shows up (true) or the child closes the terminal
    /// (false).
    fn wait_for(&mut self, needle: &[u8]) -> bool {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if self
                .seen
                .windows(needle.len())
                .any(|window| window == needle)
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) => self.seen.extend(chunk),
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                    "no {needle:?} within {DEADLINE:?}; saw {:?}",
                    String::from_utf8_lossy(&self.seen)
                ),
            }
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen).into_owned()
    }
}

impl Drop for OnPty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn plain_bacnet_on_a_terminal_opens_the_tui() {
    for no_tui_env in [None, Some(""), Some("0")] {
        let mut pty = OnPty::spawn(&[], no_tui_env);
        assert!(
            pty.wait_for(ALTERNATE_SCREEN),
            "{no_tui_env:?}: {}",
            pty.text()
        );
        assert!(pty.child.try_wait().unwrap().is_none(), "the TUI exited");
    }
}

#[test]
fn no_tui_on_a_terminal_prints_the_usage_and_exits_2() {
    for (args, no_tui_env) in [(&["--no-tui"][..], None), (&[][..], Some("1"))] {
        let mut pty = OnPty::spawn(args, no_tui_env);
        // Clap styles the usage on a terminal, so look for the words alone.
        assert!(
            pty.wait_for(b"the terminal UI won't open"),
            "{args:?}: {}",
            pty.text()
        );
        let status = pty.child.wait().unwrap();
        assert_eq!(status.code(), Some(2), "{args:?} {no_tui_env:?}");
        // Whatever else it wrote, it never entered the alternate screen.
        assert!(
            !pty.wait_for(ALTERNATE_SCREEN),
            "{args:?} opened the TUI: {}",
            pty.text()
        );
        assert!(pty.text().contains("Usage:"), "{}", pty.text());
    }
}
