//! Synchronous GENERIC_READ serial ownership; all native calls stay here.
use crate::mstp::monitor::MonitorAdmissionError;
use bacnet_types::error::Error;
use std::io::{self, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
use windows_sys::Win32::Devices::Communication::*;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, ReadFile,
};

#[derive(Debug)]
pub(crate) struct Input {
    handle: Option<OwnedHandle>,
}
impl Input {
    pub(crate) fn open(
        path: &str,
        baud: u32,
        rts: Option<bool>,
    ) -> Result<Self, MonitorAdmissionError> {
        if baud == 0 {
            return Err(MonitorAdmissionError::unopened(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero baud rate",
            )));
        }
        let path = if path.starts_with(r"\\.\") {
            path.to_owned()
        } else {
            format!(r"\\.\{path}")
        };
        let mut input = Self {
            handle: Some(open_readonly(&path).map_err(MonitorAdmissionError::unopened)?),
        };
        if let Err(error) = configure(&mut Native(input.handle()), baud, rts) {
            return Err(MonitorAdmissionError::after_close(
                Error::Transport(error),
                input.close(),
            ));
        }
        Ok(input)
    }
    fn handle(&self) -> HANDLE {
        self.handle.as_ref().expect("live input").as_raw_handle()
    }
    pub(crate) fn close(&mut self) -> io::Result<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let raw = handle.into_raw_handle();
        // SAFETY: exactly one owned handle is transferred and closed once.
        check(unsafe { CloseHandle(raw) }).map_err(|error| phase(error, "CloseHandle"))
    }
}
impl Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut count = 0;
        let length = bytes.len().min(u32::MAX as usize) as u32;
        // SAFETY: synchronous handle, valid mutable buffer, count out-pointer;
        // null OVERLAPPED is required by this non-overlapped open.
        check(unsafe {
            ReadFile(
                self.handle(),
                bytes.as_mut_ptr(),
                length,
                &mut count,
                std::ptr::null_mut(),
            )
        })
        .map_err(|error| phase(error, "ReadFile"))?;
        if count == 0 {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "serial first-byte wait expired",
            ))
        } else {
            Ok(count as usize)
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn open_readonly(path: &str) -> io::Result<OwnedHandle> {
    if path.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in serial path",
        ));
    }
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path; no security/template pointers. Read-only,
    // exclusive, existing, synchronous: no write access or OVERLAPPED flag.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(phase(
            io::Error::last_os_error(),
            "CreateFileW GENERIC_READ",
        ));
    }
    // SAFETY: CreateFileW returned one valid uniquely owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}
fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// This seam checks the same configuration sequence without claiming a COM driver.
trait Comm {
    fn state(&mut self) -> io::Result<DCB>;
    fn set_state(&mut self, state: &DCB) -> io::Result<()>;
    fn timeouts(&mut self, timeouts: &COMMTIMEOUTS) -> io::Result<()>;
    fn rts(&mut self, level: bool) -> io::Result<()>;
    fn purge(&mut self) -> io::Result<()>;
}
fn configure(api: &mut impl Comm, baud: u32, rts: Option<bool>) -> io::Result<()> {
    let mut state = api.state()?;
    state.DCBlength = std::mem::size_of::<DCB>() as u32;
    state.BaudRate = baud;
    // fBinary only: no parity/CTS/DSR/XON/XOFF filtering, null discard,
    // error replacement or abort. DTR and default RTS remain disabled.
    state._bitfield = 1 | if rts == Some(true) { 1 << 12 } else { 0 };
    state.ByteSize = 8;
    state.Parity = NOPARITY;
    state.StopBits = ONESTOPBIT;
    state.XonChar = 0x11;
    state.XoffChar = 0x13;
    api.set_state(&state)?;
    let actual = api.state()?;
    if actual.BaudRate != baud
        || actual.ByteSize != 8
        || actual.Parity != NOPARITY
        || actual.StopBits != ONESTOPBIT
        || actual._bitfield & 0x7fff != state._bitfield
    {
        return Err(io::Error::other(
            "COM driver did not retain raw 8N1 receive settings or requested baud",
        ));
    }
    api.timeouts(&COMMTIMEOUTS {
        ReadIntervalTimeout: u32::MAX,
        ReadTotalTimeoutMultiplier: u32::MAX,
        ReadTotalTimeoutConstant: 100,
        WriteTotalTimeoutMultiplier: 0,
        WriteTotalTimeoutConstant: 0,
    })?;
    if let Some(level) = rts {
        api.rts(level)?;
    }
    api.purge()
}
struct Native(HANDLE);
impl Comm for Native {
    fn state(&mut self) -> io::Result<DCB> {
        let mut state = DCB {
            DCBlength: std::mem::size_of::<DCB>() as u32,
            ..Default::default()
        };
        // SAFETY: initialized DCB with correct length; live borrowed handle.
        check(unsafe { GetCommState(self.0, &mut state) })
            .map_err(|error| phase(error, "GetCommState"))?;
        Ok(state)
    }
    fn set_state(&mut self, state: &DCB) -> io::Result<()> {
        // SAFETY: live handle and valid initialized DCB.
        check(unsafe { SetCommState(self.0, state) }).map_err(|error| phase(error, "SetCommState"))
    }
    fn timeouts(&mut self, timeouts: &COMMTIMEOUTS) -> io::Result<()> {
        // SAFETY: live handle and initialized native timeout struct.
        check(unsafe { SetCommTimeouts(self.0, timeouts) })
            .map_err(|error| phase(error, "SetCommTimeouts"))
    }
    fn rts(&mut self, level: bool) -> io::Result<()> {
        // SAFETY: live COM handle and supported RTS function selector.
        check(unsafe { EscapeCommFunction(self.0, if level { SETRTS } else { CLRRTS }) })
            .map_err(|error| phase(error, "set RTS"))
    }
    fn purge(&mut self) -> io::Result<()> {
        // SAFETY: live handle; purge input only, without any transmit request.
        check(unsafe { PurgeComm(self.0, PURGE_RXCLEAR) })
            .map_err(|error| phase(error, "PurgeComm input"))
    }
}

#[cfg(test)]
mod tests;

fn phase(error: io::Error, phase: &str) -> io::Error {
    crate::mstp::monitor::context(error, phase.into())
}
