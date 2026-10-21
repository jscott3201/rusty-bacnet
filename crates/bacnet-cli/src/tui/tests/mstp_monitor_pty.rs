//! The monitor and the join on a real pseudo-terminal (#970), Linux only:
//! on macOS the serial backend's baud-rate ioctl fails on a
//! pseudo-terminal (see `tests/cli_mstp.rs`). Real time, the system's
//! serial ports: the monitor opens the port listen-only by name, and the
//! participant opens it by name again after the monitor has closed it.

use bacnet_transport::mstp::SerialPort;
use bacnet_types::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_serial::{SerialPort as _, SerialStream};

use super::mstp_monitor::{join_and_discover, peer_server, propose, Run};
use super::*;
use crate::tui::monitor::Lines;

/// The peer's end of the pair, as a serial port.
struct Line(Mutex<SerialStream>);

impl SerialPort for Line {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        let mut line = self.0.lock().await;
        line.write_all(data).await.map_err(Error::Transport)
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.0
            .lock()
            .await
            .read(buf)
            .await
            .map_err(Error::Transport)
    }
}

/// Station 2 is the sole master on the line: the monitor sees its polls,
/// the wizard proposes 3, and the joined station finds it with a Who-Is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_monitor_hands_a_pseudo_terminal_to_the_participant() {
    let (peer_end, tui_end) = SerialStream::pair().unwrap();
    let path = tui_end.name().expect("the pseudo-terminal's path");
    let mut server = peer_server(Line(Mutex::new(peer_end))).await;
    let mut run = Run::on(Lines::System, &path, false);
    run.listen_for(Duration::ZERO).await;
    run.press(KeyCode::Char(':')).await;
    type_text(&mut run.app, "mstp listen 2");
    run.press(KeyCode::Enter).await;
    let settings = propose(&mut run).await;
    assert_eq!((settings.mac, settings.max_master), (3, 10));
    assert_eq!(settings.port, path);
    join_and_discover(&mut run, &settings, false).await;
    assert_eq!(run.quit().await, b"");
    server.stop().await.unwrap();
    // Held to the end: on Linux the peer's end fails with EIO while no
    // process holds this one.
    drop(tui_end);
}
