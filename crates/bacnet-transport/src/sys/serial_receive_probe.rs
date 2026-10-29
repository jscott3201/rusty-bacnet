//! Native assertions and deliberate write probes, only compiled for tests.
use std::io;
use std::os::fd::AsRawFd;

pub(crate) fn assert_readonly(fd: &impl AsRawFd) {
    let fd = fd.as_raw_fd();
    // SAFETY: test borrows a live fd; F_GETFL takes no third argument.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert_ne!(flags, -1);
    assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
    assert_ne!(flags & libc::O_NONBLOCK, 0);
    // SAFETY: F_GETFD takes no third argument and the fd is live.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    assert_ne!(flags & libc::FD_CLOEXEC, 0);
}
pub(crate) fn denied_write(fd: &impl AsRawFd) {
    // SAFETY: deliberate test-only denied write using a live fd and valid slice.
    let result = unsafe { libc::write(fd.as_raw_fd(), b"denied".as_ptr().cast(), 6) };
    assert_eq!(result, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
}
pub(crate) fn speed(fd: &impl AsRawFd) -> u32 {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: valid zero-initialized output storage.
        let mut termios: libc::termios2 = unsafe { std::mem::zeroed() };
        // SAFETY: correctly sized writable output and live descriptor.
        let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TCGETS2, &mut termios) };
        assert_eq!(result, 0);
        termios.c_ispeed
    }
    #[cfg(not(target_os = "linux"))]
    {
        // SAFETY: valid zero-initialized output storage.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: correctly sized writable output and live descriptor.
        assert_eq!(unsafe { libc::tcgetattr(fd.as_raw_fd(), &mut termios) }, 0);
        // SAFETY: initialized termios, read only.
        unsafe { libc::cfgetispeed(&termios) as u32 }
    }
}

/// Wait until `file` is readable, registering it with Tokio's reactor only
/// for the wait.
pub(crate) async fn readable(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsFd;
    // SAFETY: the `AsyncFd` holds a `BorrowedFd` of `file`, so the file can't
    // be closed or replaced while it is registered, and the `AsyncFd` drops,
    // deregistering, before this call returns and the borrow ends.
    let registered = unsafe {
        tokio::io::unix::AsyncFd::register_with_interest(
            file.as_fd(),
            tokio::io::Interest::READABLE,
        )
    }
    .map_err(|error| error.into_parts().1)?;
    drop(registered.readable().await?);
    Ok(())
}

pub(crate) fn queued(fd: &impl AsRawFd) -> usize {
    let mut count: libc::c_int = 0;
    // SAFETY: FIONREAD writes one integer through a valid pointer; fd is live.
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD as _, &mut count) };
    assert_eq!(result, 0);
    count as usize
}

/// Seed or inspect the inherited terminal hangup policy on a live test fd.
pub(crate) fn hupcl(fd: &impl AsRawFd, set: Option<bool>) -> bool {
    // SAFETY: zeroed termios is writable storage for tcgetattr.
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: live borrowed fd and correctly sized output.
    assert_eq!(unsafe { libc::tcgetattr(fd.as_raw_fd(), &mut settings) }, 0);
    if let Some(set) = set {
        if set {
            settings.c_cflag |= libc::HUPCL;
        } else {
            settings.c_cflag &= !libc::HUPCL;
        }
        // SAFETY: valid initialized settings and live borrowed fd.
        let result = unsafe { libc::tcsetattr(fd.as_raw_fd(), libc::TCSANOW, &settings) };
        assert_eq!(result, 0);
    }
    settings.c_cflag & libc::HUPCL != 0
}
