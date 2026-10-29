//! Native baud and raw receive configuration, with no rate substitution.
use super::serial_receive_unix::check;
use std::io;
use std::os::fd::RawFd;

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(any(target_arch = "powerpc", target_arch = "powerpc64"))
))]
pub(super) fn configure(fd: RawFd, baud: u32) -> io::Result<()> {
    // SAFETY: all-zero termios2 is valid storage for TCGETS2 to initialize.
    let mut settings: libc::termios2 = unsafe { std::mem::zeroed() };
    // SAFETY: settings is writable and sized for TCGETS2; fd is borrowed live.
    check(unsafe { libc::ioctl(fd, libc::TCGETS2, &mut settings) })?;
    let hupcl = settings.c_cflag & libc::HUPCL;
    settings.c_iflag = 0;
    settings.c_oflag = 0;
    settings.c_lflag = 0;
    settings.c_cflag = hupcl | libc::BOTHER | libc::CS8 | libc::CREAD | libc::CLOCAL;
    settings.c_ispeed = baud;
    settings.c_ospeed = baud;
    settings.c_cc[libc::VMIN] = 1;
    settings.c_cc[libc::VTIME] = 0;
    // SAFETY: ioctl reads the initialized termios2 settings.
    check(unsafe { libc::ioctl(fd, libc::TCSETS2, &settings) })?;
    // SAFETY: ioctl writes to initialized, correctly sized settings.
    check(unsafe { libc::ioctl(fd, libc::TCGETS2, &mut settings) })?;
    if settings.c_ispeed != baud
        || settings.c_ospeed != baud
        || settings.c_iflag != 0
        || settings.c_lflag != 0
        || settings.c_oflag != 0
        || settings.c_cflag
            & (libc::CSIZE
                | libc::PARENB
                | libc::CSTOPB
                | libc::CRTSCTS
                | libc::CREAD
                | libc::CLOCAL
                | libc::HUPCL)
            != (libc::CS8 | libc::CREAD | libc::CLOCAL | hupcl)
    {
        return Err(io::Error::other(
            "serial driver did not retain raw 8N1 receive settings or requested baud",
        ));
    }
    Ok(())
}

#[cfg(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
pub(super) fn configure(fd: RawFd, baud: u32) -> io::Result<()> {
    // SAFETY: zero is valid storage, initialized fully by tcgetattr below.
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: writable termios and borrowed live descriptor.
    check(unsafe { libc::tcgetattr(fd, &mut settings) })?;
    let hupcl = settings.c_cflag & libc::HUPCL;
    settings.c_iflag = 0;
    settings.c_oflag = 0;
    settings.c_lflag = 0;
    settings.c_cflag = hupcl | libc::CS8 | libc::CREAD | libc::CLOCAL;
    settings.c_cc[libc::VMIN] = 1;
    settings.c_cc[libc::VTIME] = 0;
    #[cfg(target_vendor = "apple")]
    let custom = !matches!(
        baud,
        50 | 75
            | 110
            | 134
            | 150
            | 200
            | 300
            | 600
            | 1200
            | 1800
            | 2400
            | 4800
            | 9600
            | 19200
            | 38400
            | 57600
            | 115200
            | 230400
    );
    #[cfg(not(target_vendor = "apple"))]
    let custom = false;
    let speed = if custom { 9600 } else { baud } as libc::speed_t;
    // SAFETY: both setters mutate a valid local termios.
    check(unsafe { libc::cfsetispeed(&mut settings, speed) })?;
    // SAFETY: same local termios, native speed value.
    check(unsafe { libc::cfsetospeed(&mut settings, speed) })?;
    // SAFETY: tcsetattr borrows initialized settings and a live descriptor.
    check(unsafe { libc::tcsetattr(fd, libc::TCSANOW, &settings) })?;
    #[cfg(target_vendor = "apple")]
    if custom {
        // Apple's IOSSIOSPEED accepts a speed_t; termios does not read it back.
        const IOSSIOSPEED: libc::c_ulong = 0x8008_5402;
        let requested = baud as libc::speed_t;
        // SAFETY: ioctl reads one native speed_t from a valid pointer.
        check(unsafe { libc::ioctl(fd, IOSSIOSPEED, &requested) })?;
    }
    // SAFETY: correctly sized writable termios and live descriptor.
    check(unsafe { libc::tcgetattr(fd, &mut settings) })?;
    // SAFETY: both getters only read the initialized termios.
    let actual = unsafe { (libc::cfgetispeed(&settings), libc::cfgetospeed(&settings)) };
    if (!custom && actual != (speed, speed))
        || settings.c_iflag != 0
        || settings.c_lflag != 0
        || settings.c_oflag != 0
        || settings.c_cflag
            & (libc::CSIZE
                | libc::PARENB
                | libc::CSTOPB
                | libc::CRTSCTS
                | libc::CREAD
                | libc::CLOCAL
                | libc::HUPCL)
            != (libc::CS8 | libc::CREAD | libc::CLOCAL | hupcl)
    {
        return Err(io::Error::other(
            "serial driver did not retain raw 8N1 receive settings or requested baud",
        ));
    }
    Ok(())
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    all(
        any(target_os = "linux", target_os = "android"),
        not(any(target_arch = "powerpc", target_arch = "powerpc64"))
    )
)))]
pub(super) fn configure(_: RawFd, _: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native read-only serial baud configuration is unavailable on this target",
    ))
}
