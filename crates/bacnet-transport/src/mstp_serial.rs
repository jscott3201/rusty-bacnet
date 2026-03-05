//! Real serial port adapter for MS/TP using `tokio-serial`.
//!
//! RS-485 is half-duplex — this assumes the hardware (USB-RS485 adapter)
//! handles direction switching automatically (which most modern adapters do).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_serial::SerialStream;

use bacnet_types::error::Error;

use crate::mstp::SerialPort;

/// Configuration for a serial port connection.
pub struct SerialConfig {
    /// Serial port device name (e.g., "/dev/ttyUSB0" on Linux, "COM3" on Windows).
    pub port_name: String,
    /// Baud rate. Common MS/TP values: 9600, 19200, 38400, 76800.
    pub baud_rate: u32,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            port_name: "/dev/ttyUSB0".into(),
            baud_rate: 76800,
        }
    }
}

/// A real serial port implementing the MS/TP [`SerialPort`] trait.
///
/// Wraps `tokio_serial::SerialStream` for async RS-485 I/O.
pub struct TokioSerialPort {
    inner: Mutex<SerialStream>,
}

impl TokioSerialPort {
    /// Open a serial port with the given configuration.
    pub fn open(config: &SerialConfig) -> Result<Self, Error> {
        let builder = tokio_serial::new(&config.port_name, config.baud_rate);
        let stream = SerialStream::open(&builder)
            .map_err(|e| Error::Encoding(format!("Serial open failed: {e}")))?;
        Ok(Self {
            inner: Mutex::new(stream),
        })
    }
}

impl SerialPort for TokioSerialPort {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        let mut stream = self.inner.lock().await;
        stream
            .write_all(data)
            .await
            .map_err(|e| Error::Encoding(format!("Serial write failed: {e}")))
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut stream = self.inner.lock().await;
        stream
            .read(buf)
            .await
            .map_err(|e| Error::Encoding(format!("Serial read failed: {e}")))
    }
}
