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

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_serial::SerialStream;

use bacnet_types::error::Error;

use crate::mstp::SerialPort;

#[cfg(any(target_os = "linux", test))]
mod kernel_rs485;

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
    inner: Arc<Mutex<SerialStream>>,
}

impl TokioSerialPort {
    /// Open a serial port with the given configuration.
    pub fn open(config: &SerialConfig) -> Result<Self, Error> {
        let builder = tokio_serial::new(&config.port_name, config.baud_rate);
        let stream = SerialStream::open(&builder)
            .map_err(|e| Error::Encoding(format!("Serial open failed: {e}")))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(stream)),
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
    ///
    /// Delays must be multiples of 1000 microseconds (zero is valid), because
    /// Linux represents them in whole milliseconds. Other values are rejected
    /// before any ioctl; delays are never rounded.
    ///
    /// After setting the configuration, `TIOCGRS485` must confirm RS-485 is
    /// enabled with the requested RTS polarity and delays. A readback failure
    /// or driver-sanitized mismatch returns an error even though the hardware
    /// configuration may already have changed. No rollback or retry is attempted.
    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    pub fn enable_kernel_rs485(
        &self,
        invert_rts: bool,
        delay_before_send_us: u32,
        delay_after_send_us: u32,
    ) -> Result<(), Error> {
        use std::os::unix::io::AsRawFd;

        #[cfg(not(any(target_arch = "sparc", target_arch = "sparc64")))]
        use libc::{TIOCGRS485, TIOCSRS485};
        // libc 0.2.189 does not export these on SPARC. Linux v6.12 UAPI:
        // arch/sparc/include/uapi/asm/{ioctls,ioctl}.h encodes a 32-byte
        // serial_rs485 with _IOR('T', 0x41, ...) and _IOWR('T', 0x42, ...).
        #[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
        const TIOCGRS485: libc::c_ulong = 0x4020_5441;
        #[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
        const TIOCSRS485: libc::c_ulong = 0xc020_5442;

        let stream = self.inner.try_lock().map_err(|_| {
            Error::Encoding("Cannot enable RS-485: serial port is in use".to_string())
        })?;

        kernel_rs485::configure(
            invert_rts,
            delay_before_send_us,
            delay_after_send_us,
            |request, config| {
                let request = match request {
                    kernel_rs485::Request::Set => TIOCSRS485,
                    kernel_rs485::Request::Get => TIOCGRS485,
                };
                // SAFETY: The lock guard keeps the serial stream and its fd alive.
                // `config` is a writable, initialized 32-byte Linux serial_rs485
                // structure, and the pointer remains valid for this ioctl call.
                let ret = unsafe { libc::ioctl(stream.as_raw_fd(), request, config as *mut _) };
                if ret < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            },
        )
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

    #[cfg(unix)]
    async fn drain(&self) -> Result<(), Error> {
        // The synchronous Write implementation reaches tcdrain on Unix and
        // propagates its result. Do not run this blocking wait on a Tokio worker.
        // The owned guard keeps the stream alive and exclusive even if the
        // awaiting future is cancelled while the blocking task is running.
        let mut stream = self.inner.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || std::io::Write::flush(&mut *stream))
            .await
            .map_err(|e| Error::Encoding(format!("Serial drain task failed: {e}")))?
            .map_err(|e| Error::Encoding(format!("Serial drain failed: {e}")))
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

#[cfg(any(feature = "serial-gpio", test))]
trait DirectionControl: Send + Sync {
    fn set_tx_mode(&self) -> Result<(), Error>;
    fn set_rx_mode(&self) -> Result<(), Error>;
}

/// Serializes direction changes with I/O and remembers unfinished transmission
/// across errors or cancellation. No RX transition is safe until drain succeeds.
#[cfg(any(feature = "serial-gpio", test))]
struct SoftwareDirection {
    tx_pending: Mutex<bool>,
    post_tx_delay_us: u64,
}

#[cfg(any(feature = "serial-gpio", test))]
impl SoftwareDirection {
    fn new(post_tx_delay_us: u64) -> Self {
        Self {
            tx_pending: Mutex::new(false),
            post_tx_delay_us,
        }
    }

    async fn finish_transmit(
        &self,
        inner: &impl SerialPort,
        direction: &impl DirectionControl,
        tx_pending: &mut bool,
    ) -> Result<(), Error> {
        if *tx_pending {
            inner.drain().await?;
            // This is a transceiver guard interval AFTER confirmed completion,
            // never an estimate of how long queued bytes take to reach the wire.
            if self.post_tx_delay_us > 0 {
                tokio::time::sleep(tokio::time::Duration::from_micros(self.post_tx_delay_us)).await;
            }
            direction.set_rx_mode()?;
            *tx_pending = false;
        }
        Ok(())
    }

    async fn write(
        &self,
        inner: &impl SerialPort,
        direction: &impl DirectionControl,
        data: &[u8],
    ) -> Result<(), Error> {
        let mut tx_pending = self.tx_pending.lock().await;
        self.finish_transmit(inner, direction, &mut tx_pending)
            .await?;
        if let Err(error) = direction.set_tx_mode() {
            // No write was attempted, so restoring RX cannot truncate output.
            direction.set_rx_mode()?;
            return Err(error);
        }
        *tx_pending = true;
        let result = inner.write(data).await;
        // A failed write can still have queued bytes. Drain those before RX too.
        self.finish_transmit(inner, direction, &mut tx_pending)
            .await?;
        result
    }

    async fn read(
        &self,
        inner: &impl SerialPort,
        direction: &impl DirectionControl,
        buf: &mut [u8],
    ) -> Result<usize, Error> {
        let mut tx_pending = self.tx_pending.lock().await;
        self.finish_transmit(inner, direction, &mut tx_pending)
            .await?;
        inner.read(buf).await
    }
}

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
/// each drained write. The inner port must implement [`SerialPort::drain`].
/// If drain fails or a write is cancelled, DE remains asserted until a later
/// read/write can confirm completion and restore RX. Dropping the wrapper is not
/// an asynchronous drain; finish pending I/O before releasing the GPIO resource.
#[cfg(feature = "serial-gpio")]
pub struct GpioDirectionPort<S: SerialPort> {
    inner: S,
    gpio: std::sync::Mutex<gpiocdev::Request>,
    line: u32,
    active_high: bool,
    direction: SoftwareDirection,
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
    /// After the inner port confirms transmit completion, the wrapper waits
    /// this additional transceiver guard interval before switching to RX.
    /// Zero adds no guard interval. This delay is not a substitute for drain
    /// and must fit the link's driver-release timing budget.
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
            direction: SoftwareDirection::new(post_tx_delay_us),
        })
    }
}

#[cfg(feature = "serial-gpio")]
impl<S: SerialPort> DirectionControl for GpioDirectionPort<S> {
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
        self.direction.write(&self.inner, self, data).await
    }

    async fn drain(&self) -> Result<(), Error> {
        let mut tx_pending = self.direction.tx_pending.lock().await;
        self.direction
            .finish_transmit(&self.inner, self, &mut tx_pending)
            .await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.direction.read(&self.inner, self, buf).await
    }
}

#[cfg(test)]
mod tests;
