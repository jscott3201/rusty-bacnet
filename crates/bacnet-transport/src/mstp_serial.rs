//! Real serial port adapters for MS/TP using `tokio-serial`.
//!
//! Provides three RS-485 direction control modes:
//!
//! - **Auto** (`TokioSerialPort`): Hardware handles direction switching.
//!   Works with USB-RS485 adapters (FTDI, CH340, CP2102) that toggle
//!   direction automatically.
//!
//! - **Kernel RS-485** (`TokioSerialPort::enable_kernel_rs485`): Uses the
//!   Linux `TIOCSRS485` ioctl so the kernel toggles the UART's RTS pin
//!   around each transmission. Zero userspace overhead. Requires DE/RE
//!   wired to the UART's RTS pin.
//!
//! - **GPIO** (`GpioDirectionPort`): Toggles an arbitrary GPIO pin for
//!   DE/RE control via the Linux GPIO character device (`/dev/gpiochipN`).
//!   Use this for RS-485 hats (like the Seeed Studio RS-485 Shield) where
//!   DE/RE is wired to a GPIO pin rather than the UART's RTS.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_serial::SerialStream;

use bacnet_types::error::Error;

use crate::mstp::SerialPort;

/// Configuration for a serial port connection.
pub struct SerialConfig {
    /// Serial port device name (e.g., "/dev/ttyUSB0" on Linux, "/dev/cu.usbserial-xxx" on macOS).
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
/// Wraps `tokio_serial::SerialStream` for async RS-485 I/O. By default,
/// assumes the hardware handles direction switching automatically (USB
/// RS-485 adapters). On Linux, call [`enable_kernel_rs485`](Self::enable_kernel_rs485)
/// to use kernel-managed RTS direction control.
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

    /// Enable Linux kernel RS-485 mode via `TIOCSRS485` ioctl.
    ///
    /// The kernel will automatically toggle the UART's RTS pin to control
    /// the RS-485 transceiver direction. This is zero-overhead — no
    /// userspace GPIO toggling needed. Requires DE/RE wired to the UART's
    /// RTS pin (e.g., GPIO17 on Raspberry Pi).
    ///
    /// # Parameters
    /// - `invert_rts`: If true, RTS is LOW during transmission (for
    ///   transceivers with active-low DE).
    /// - `delay_before_send_us`: Microseconds to wait after asserting RTS
    ///   before transmitting. Covers transceiver enable time.
    /// - `delay_after_send_us`: Microseconds to wait after the last byte
    ///   before deasserting RTS. Covers last-byte drain time.
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    pub fn enable_kernel_rs485(
        &self,
        invert_rts: bool,
        delay_before_send_us: u32,
        delay_after_send_us: u32,
    ) -> Result<(), Error> {
        use std::os::unix::io::AsRawFd;

        let stream = self.inner.try_lock().map_err(|_| {
            Error::Encoding("Cannot enable RS-485: serial port is in use".to_string())
        })?;

        // struct serial_rs485 { u32 flags, u32 delay_rts_before_send, u32 delay_rts_after_send, u32[5] padding }
        const SER_RS485_ENABLED: u32 = 1;
        const SER_RS485_RTS_ON_SEND: u32 = 1 << 1;
        const SER_RS485_RTS_AFTER_SEND: u32 = 1 << 2;

        let mut flags = SER_RS485_ENABLED;
        if invert_rts {
            flags |= SER_RS485_RTS_AFTER_SEND;
        } else {
            flags |= SER_RS485_RTS_ON_SEND;
        }

        let mut buf = [0u32; 8];
        buf[0] = flags;
        buf[1] = delay_before_send_us;
        buf[2] = delay_after_send_us;

        // TIOCSRS485 = 0x542F
        // SAFETY: `stream` is a live `TokioSerial` instance whose fd is open for the duration
        // of the lock guard; `buf` is a `[u32; 8]` stack array sized to match the kernel's
        // `serial_rs485` struct (8 * u32 = 32 bytes); pointer is valid for the call.
        let ret = unsafe { libc::ioctl(stream.as_raw_fd(), 0x542F, buf.as_mut_ptr()) };
        if ret < 0 {
            return Err(Error::Encoding(format!(
                "TIOCSRS485 ioctl failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        tracing::info!("Kernel RS-485 mode enabled (invert_rts={invert_rts})");
        Ok(())
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

// ---------------------------------------------------------------------------
// GPIO direction control wrapper
// ---------------------------------------------------------------------------

/// RS-485 direction control via a GPIO pin on the Linux GPIO character device.
///
/// Wraps any [`SerialPort`] implementation and toggles a GPIO pin for
/// DE/RE (Driver Enable / Receiver Enable) control around each write.
///
/// # Usage
///
/// ```no_run
/// use bacnet_transport::mstp_serial::{GpioDirectionPort, TokioSerialPort, SerialConfig};
///
/// let serial = TokioSerialPort::open(&SerialConfig {
///     port_name: "/dev/ttyS0".into(),
///     baud_rate: 76800,
/// }).unwrap();
///
/// // Seeed Studio RS-485 Shield: GPIO18 on /dev/gpiochip0, active-high
/// let port = GpioDirectionPort::new(serial, "/dev/gpiochip0", 18, true).unwrap();
/// ```
///
/// The pin is set to receive mode (DE deasserted) on creation and after
/// each write. This ensures the bus defaults to listening.
#[cfg(feature = "serial-gpio")]
pub struct GpioDirectionPort<S: SerialPort> {
    inner: S,
    gpio: std::sync::Mutex<gpiocdev::Request>,
    line: u32,
    active_high: bool,
    /// Baud-rate-dependent delay after flush to ensure the last byte
    /// has left the UART shift register before switching to RX mode.
    post_tx_delay_us: u64,
}

#[cfg(feature = "serial-gpio")]
impl<S: SerialPort> GpioDirectionPort<S> {
    /// Create a new GPIO direction-controlled serial port.
    ///
    /// # Parameters
    /// - `inner`: The underlying serial port for data I/O.
    /// - `gpio_chip`: Path to the GPIO chip device (e.g., "/dev/gpiochip0").
    /// - `line`: GPIO line number for DE/RE control (e.g., 18).
    /// - `active_high`: If true, GPIO HIGH enables the transmitter (most
    ///   common — MAX485 DE pin is active-high). If false, GPIO LOW enables TX.
    pub fn new(inner: S, gpio_chip: &str, line: u32, active_high: bool) -> Result<Self, Error> {
        Self::with_post_tx_delay(inner, gpio_chip, line, active_high, 0)
    }

    /// Create with an explicit post-TX delay in microseconds.
    ///
    /// After flushing the serial port, the wrapper waits this long before
    /// switching back to receive mode. This covers the time for the last
    /// byte to leave the UART's shift register. At 76800 baud, one byte
    /// takes ~130us. A delay of 200-500us is typically safe.
    ///
    /// If set to 0, no additional delay is added (suitable when the UART
    /// driver's flush fully drains the hardware FIFO).
    pub fn with_post_tx_delay(
        inner: S,
        gpio_chip: &str,
        line: u32,
        active_high: bool,
        post_tx_delay_us: u64,
    ) -> Result<Self, Error> {
        use gpiocdev::line::Value;

        // Start in RX mode (DE deasserted).
        let rx_value = if active_high {
            Value::Inactive
        } else {
            Value::Active
        };

        let request = gpiocdev::Request::builder()
            .on_chip(gpio_chip)
            .with_line(line)
            .as_output(rx_value)
            .with_consumer("bacnet-mstp")
            .request()
            .map_err(|e| {
                Error::Encoding(format!(
                    "GPIO request failed for {gpio_chip} line {line}: {e}"
                ))
            })?;

        tracing::info!(
            "GPIO direction control: {gpio_chip} line {line} (active_high={active_high})"
        );

        Ok(Self {
            inner,
            gpio: std::sync::Mutex::new(request),
            line,
            active_high,
            post_tx_delay_us,
        })
    }

    /// Set the transceiver to transmit mode (DE asserted).
    fn set_tx_mode(&self) -> Result<(), Error> {
        use gpiocdev::line::Value;
        let value = if self.active_high {
            Value::Active
        } else {
            Value::Inactive
        };
        self.gpio
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_value(self.line, value)
            .map_err(|e| Error::Encoding(format!("GPIO set TX mode failed: {e}")))
    }

    /// Set the transceiver to receive mode (DE deasserted).
    fn set_rx_mode(&self) -> Result<(), Error> {
        use gpiocdev::line::Value;
        let value = if self.active_high {
            Value::Inactive
        } else {
            Value::Active
        };
        self.gpio
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_value(self.line, value)
            .map_err(|e| Error::Encoding(format!("GPIO set RX mode failed: {e}")))
    }
}

#[cfg(feature = "serial-gpio")]
impl<S: SerialPort> SerialPort for GpioDirectionPort<S> {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        // Switch to TX mode before writing.
        self.set_tx_mode()?;

        let result = self.inner.write(data).await;

        // Post-TX delay to let the last byte leave the shift register.
        if self.post_tx_delay_us > 0 {
            tokio::time::sleep(tokio::time::Duration::from_micros(self.post_tx_delay_us)).await;
        }

        // Always switch back to RX mode, even on write error.
        self.set_rx_mode()?;

        result
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        // Already in RX mode — just delegate.
        self.inner.read(buf).await
    }
}
