//! Where the TUI's MS/TP sessions open their serial port: the system's
//! ports, or in tests an in-memory line.
//!
//! The monitor gets only a reader, so nothing it holds can transmit. The
//! participant gets a [`CliSerial`]. A port has one owner at a time: the
//! system opens a serial port for exclusive use, and the test line refuses
//! the participant while the monitor's reader is alive, so a join that
//! didn't wait for the monitor to close the port fails in tests too.

use bacnet_client::client::BACnetClient;
use bacnet_transport::mstp::monitor::{MonitorAdmissionError, MonitorRead};
use bacnet_transport::mstp::{Echo, MstpDiagnostics, MstpThreadConfig, MstpTransport, SerialPort};
use bacnet_types::error::Error;

use crate::transport::TransportArgs;

/// What the monitor reads from: a reader and nothing else.
pub(crate) type MonitorReader = Box<dyn MonitorRead>;

/// The serial port of a participant session.
pub(crate) enum CliSerial {
    /// A system serial port.
    #[cfg(feature = "serial")]
    System(crate::mstp::Serial),
    /// One end of an in-memory line.
    #[cfg(test)]
    Test(bacnet_transport::mstp::LoopbackSerial),
}

// Without `serial`, outside tests, the enum has no variant to use them.
#[cfg_attr(not(any(feature = "serial", test)), allow(unused_variables))]
impl SerialPort for CliSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        match *self {
            #[cfg(feature = "serial")]
            Self::System(ref port) => port.write(data).await,
            #[cfg(test)]
            Self::Test(ref port) => port.write(data).await,
        }
    }

    async fn drain(&self) -> Result<(), Error> {
        match *self {
            #[cfg(feature = "serial")]
            Self::System(ref port) => port.drain().await,
            #[cfg(test)]
            Self::Test(ref port) => port.drain().await,
        }
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        match *self {
            #[cfg(feature = "serial")]
            Self::System(ref port) => port.read(buf).await,
            #[cfg(test)]
            Self::Test(ref port) => port.read(buf).await,
        }
    }

    fn echo(&self) -> Echo {
        match *self {
            #[cfg(feature = "serial")]
            Self::System(ref port) => port.echo(),
            #[cfg(test)]
            Self::Test(ref port) => port.echo(),
        }
    }
}

/// Where serial ports are opened.
#[derive(Clone, Default)]
pub(crate) enum Lines {
    /// The system's serial ports (the `serial` feature).
    #[default]
    System,
    /// Reopenable rate-aware scripted scanner input.
    #[cfg(test)]
    Scan(std::sync::Arc<super::scan_fixture::ScanLine>),
    /// An in-memory line.
    #[cfg(test)]
    Test(std::sync::Arc<test_line::TestLine>),
}

impl std::fmt::Debug for Lines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => f.write_str("System"),
            #[cfg(test)]
            Self::Scan(_) => f.write_str("Scan"),
            #[cfg(test)]
            Self::Test(_) => f.write_str("Test"),
        }
    }
}

impl Lines {
    /// Open `port` at `baud` to listen only.
    pub(crate) fn open_monitor(
        &self,
        port: &str,
        baud: u32,
    ) -> Result<MonitorReader, MonitorAdmissionError> {
        match self {
            Self::System => system_monitor(port, baud),
            #[cfg(test)]
            Self::Scan(line) => line.monitor(baud),
            #[cfg(test)]
            Self::Test(line) => line.monitor(),
        }
    }

    /// Open `args.mstp`'s port and join the trunk there, the station's
    /// thread scheduled as `thread` says (`--mstp-rt-priority`,
    /// `--mstp-cpus`).
    pub(crate) async fn participant(
        &self,
        args: &TransportArgs,
        thread: &MstpThreadConfig,
    ) -> Result<(BACnetClient<MstpTransport<CliSerial>>, MstpDiagnostics), Error> {
        let mstp = args
            .mstp
            .as_ref()
            .ok_or_else(|| Error::Encoding("no MS/TP settings to join with".into()))?;
        let (serial, mode) = match self {
            Self::System => (system_serial(mstp)?, crate::mstp_thread::mode(thread)),
            #[cfg(test)]
            Self::Scan(line) => (
                line.participant()?,
                bacnet_transport::mstp::MstpExecutionMode::Tokio,
            ),
            #[cfg(test)]
            Self::Test(line) => {
                // Paused time needs the loop on the test's runtime; the
                // thread settings must still be ones the platform takes.
                use bacnet_transport::mstp::MstpExecutionMode;
                MstpExecutionMode::DedicatedThread(thread.clone()).validate()?;
                (line.participant()?, MstpExecutionMode::Tokio)
            }
        };
        crate::mstp::build_client_on(serial, mstp, args.timeout_ms, args.min_interval_ms, mode)
            .await
    }
}

#[cfg(feature = "serial")]
fn system_monitor(port: &str, baud: u32) -> Result<MonitorReader, MonitorAdmissionError> {
    use bacnet_transport::mstp_serial::{ReceiveOnlyPort, SerialConfig};
    let config = SerialConfig {
        port_name: port.to_string(),
        baud_rate: baud,
    };
    Ok(Box::new(ReceiveOnlyPort::open(&config, None)?))
}

#[cfg(not(feature = "serial"))]
fn system_monitor(_port: &str, _baud: u32) -> Result<MonitorReader, MonitorAdmissionError> {
    Err(MonitorAdmissionError::without_input(Error::Encoding(
        crate::mstp::NO_SERIAL.into(),
    )))
}

#[cfg(feature = "serial")]
fn system_serial(mstp: &crate::mstp::MstpArgs) -> Result<CliSerial, Error> {
    crate::mstp::open_serial(mstp).map(CliSerial::System)
}

#[cfg(not(feature = "serial"))]
fn system_serial(_mstp: &crate::mstp::MstpArgs) -> Result<CliSerial, Error> {
    Err(Error::Encoding(crate::mstp::NO_SERIAL.into()))
}

#[cfg(test)]
pub(crate) mod test_line {
    //! An in-memory serial port for the monitor and then a participant.

    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use bacnet_transport::mstp::LoopbackSerial;
    use bacnet_types::error::Error;
    use tokio::io::{AsyncRead, DuplexStream, ReadBuf};

    use super::{CliSerial, MonitorAdmissionError, MonitorReader};

    /// One port: the monitor's end of a scripted line, then the
    /// participant's end of a loopback pair. Each is handed out once.
    pub(crate) struct TestLine {
        monitor: Mutex<Option<MonitorReader>>,
        participant: Mutex<Option<LoopbackSerial>>,
        /// The monitor's reader is alive: the port is taken.
        held: Arc<AtomicBool>,
        pub(crate) opens: AtomicUsize,
    }

    impl TestLine {
        pub(crate) fn new(monitor: DuplexStream, participant: Option<LoopbackSerial>) -> Arc<Self> {
            let held = Arc::new(AtomicBool::new(false));
            let reader = Box::new(bacnet_transport::mstp::monitor::DropOnClose::new(Held {
                stream: monitor,
                held: Arc::clone(&held),
            }));
            Self::with_reader(Some(reader), held, participant)
        }

        pub(crate) fn with_reader(
            reader: Option<MonitorReader>,
            held: Arc<AtomicBool>,
            participant: Option<LoopbackSerial>,
        ) -> Arc<Self> {
            Arc::new(Self {
                monitor: Mutex::new(reader),
                participant: Mutex::new(participant),
                held,
                opens: AtomicUsize::new(0),
            })
        }

        pub(super) fn monitor(&self) -> Result<MonitorReader, MonitorAdmissionError> {
            let reader = self.monitor.lock().unwrap().take().ok_or_else(|| {
                MonitorAdmissionError::without_input(Error::Encoding(
                    "test line: no monitor end".into(),
                ))
            })?;
            self.held.store(true, Ordering::SeqCst);
            Ok(reader)
        }

        /// Whether the monitor's reader is still alive.
        pub(crate) fn held(&self) -> bool {
            self.held.load(Ordering::SeqCst)
        }

        /// Busy while the monitor's reader is alive, as an exclusive open
        /// of a real port would be.
        pub(super) fn participant(&self) -> Result<CliSerial, Error> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if self.held.load(Ordering::SeqCst) {
                return Err(Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::ResourceBusy,
                    "test line: the monitor still holds the port",
                )));
            }
            let serial = self.participant.lock().unwrap().take();
            serial
                .map(CliSerial::Test)
                .ok_or_else(|| Error::Encoding("test line: no participant end".into()))
        }
    }

    /// The monitor's reader; dropping it frees the port.
    struct Held {
        stream: DuplexStream,
        held: Arc<AtomicBool>,
    }

    impl AsyncRead for Held {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            self.held.store(false, Ordering::SeqCst);
        }
    }
}
