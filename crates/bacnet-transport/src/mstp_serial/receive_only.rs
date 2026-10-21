//! A serial port opened for listening only, for the MS/TP monitor.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf, ReadHalf};
use tokio_serial::{SerialPort as _, SerialStream};

use bacnet_types::error::Error;

use super::SerialConfig;

/// A serial port that can only be read: the input for a
/// [`MstpMonitor`](crate::mstp::monitor::MstpMonitor).
///
/// It keeps the read half of the port and drops the write half when it
/// opens, so nothing that holds it can transmit. It configures no kernel
/// RS-485 transmit control and requests no GPIO line, so a transceiver
/// whose driver enable is a GPIO stays as it was, normally receiving.
///
/// The port is opened for exclusive use, as for a transport, so it can't
/// be one an MS/TP transport on this host already has open. Use a second
/// adapter on the same trunk.
///
/// # Request to Send
///
/// Opening a serial port on Linux raises the modem control lines, RTS
/// among them, and on an adapter whose driver enable follows RTS with no
/// kernel RS-485 mode to manage it, a raised RTS can put the driver on the
/// line. [`open`](Self::open) can set RTS to a given level straight after
/// opening, for such an adapter: pass the level that keeps its driver off.
/// It doesn't change the port's kernel RS-485 settings, and a kernel in
/// RS-485 mode only drives RTS around transmissions, which never happen.
/// On Linux the lines go up as the port opens, before `request_to_send`
/// can lower RTS, so such an adapter's driver may be on for that brief
/// window; no open from user space avoids it.
#[derive(Debug)]
pub struct ReceiveOnlyPort {
    read: ReadHalf<SerialStream>,
}

impl ReceiveOnlyPort {
    /// Open the port at `config`'s baud rate, receive only.
    ///
    /// `request_to_send` sets the RTS line to that level once the port is
    /// open; `None` leaves it as opening the port did (see the [type
    /// docs](Self)).
    ///
    /// # Errors
    ///
    /// [`Error::Transport`] if the port can't be opened or RTS can't be set.
    pub fn open(config: &SerialConfig, request_to_send: Option<bool>) -> Result<Self, Error> {
        let builder = tokio_serial::new(&config.port_name, config.baud_rate);
        let mut stream = SerialStream::open(&builder).map_err(|error| {
            let error = std::io::Error::from(error);
            Error::Transport(std::io::Error::new(
                error.kind(),
                format!("serial open of {} failed: {error}", config.port_name),
            ))
        })?;
        if let Some(level) = request_to_send {
            stream.write_request_to_send(level).map_err(|error| {
                let error = std::io::Error::from(error);
                Error::Transport(std::io::Error::new(
                    error.kind(),
                    format!("setting RTS on {} failed: {error}", config.port_name),
                ))
            })?;
        }
        Ok(Self::from_stream(stream))
    }

    /// Keep the read half of `stream` and drop the write half.
    pub(super) fn from_stream(stream: SerialStream) -> Self {
        let (read, write) = tokio::io::split(stream);
        drop(write);
        Self { read }
    }
}

impl AsyncRead for ReceiveOnlyPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}
