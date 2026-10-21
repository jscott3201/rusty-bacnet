//! Linux kernel RS-485: the `serial_rs485` layout and the `TIOCSRS485` and
//! `TIOCGRS485` ioctls that set and read it. `mstp_serial`'s
//! `kernel_rs485::configure` holds the logic, over these types on every
//! target; the ioctl itself is Linux only.

// Linux UAPI serial_rs485; addressing features are unused and left zeroed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SerialRs485 {
    pub(crate) flags: u32,
    pub(crate) delay_rts_before_send: u32,
    pub(crate) delay_rts_after_send: u32,
    pub(crate) padding: [u32; 5],
}

// The ioctl reads and writes exactly this many octets.
const _: () = assert!(std::mem::size_of::<SerialRs485>() == 32);

/// Which RS-485 ioctl to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    /// `TIOCSRS485`: set the configuration.
    Set,
    /// `TIOCGRS485`: read it back.
    Get,
}

/// Make the RS-485 ioctl `request` on the serial port `port`, with `config`
/// as the `serial_rs485` it sets from or reads into.
#[cfg(target_os = "linux")]
pub(crate) fn rs485_ioctl(
    port: &tokio_serial::SerialStream,
    request: Request,
    config: &mut SerialRs485,
) -> std::io::Result<()> {
    #[cfg(not(any(target_arch = "sparc", target_arch = "sparc64")))]
    use libc::{TIOCGRS485, TIOCSRS485};
    // libc 0.2.189 does not export these on SPARC. Linux v6.12 UAPI:
    // arch/sparc/include/uapi/asm/{ioctls,ioctl}.h encodes a 32-byte
    // serial_rs485 with _IOR('T', 0x41, ...) and _IOWR('T', 0x42, ...).
    #[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
    const TIOCGRS485: libc::c_ulong = 0x4020_5441;
    #[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
    const TIOCSRS485: libc::c_ulong = 0xc020_5442;

    use std::os::fd::AsRawFd;

    let request = match request {
        Request::Set => TIOCSRS485,
        Request::Get => TIOCGRS485,
    };
    // SAFETY: The borrow keeps the serial stream and its fd open, and the
    // fd is a serial port's.
    // `config` is a writable, initialized 32-byte Linux serial_rs485
    // structure (the layout above, size checked at compile time), and the
    // pointer remains valid for this ioctl call, which reads or writes no
    // more than that.
    let ret = unsafe { libc::ioctl(port.as_raw_fd(), request, config as *mut _) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_serial_rs485_layout_matches_uapi() {
        use std::mem::{align_of, offset_of, size_of};

        assert_eq!(size_of::<SerialRs485>(), 32);
        assert_eq!(align_of::<SerialRs485>(), 4);
        assert_eq!(offset_of!(SerialRs485, flags), 0);
        assert_eq!(offset_of!(SerialRs485, delay_rts_before_send), 4);
        assert_eq!(offset_of!(SerialRs485, delay_rts_after_send), 8);
        assert_eq!(offset_of!(SerialRs485, padding), 12);
        assert_eq!(size_of::<[u32; 5]>(), 20);
    }
}
