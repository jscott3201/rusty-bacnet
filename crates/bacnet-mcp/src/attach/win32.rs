//! The few Win32 calls the named pipe needs that Tokio doesn't make: whose
//! process is at either end, which user a process runs as, and the
//! owner-only security descriptor. Each call is wrapped in a small safe
//! function; every handle and every buffer the system allocates has an
//! owner that frees it, on error paths too. This module is the crate's
//! only unsafe code, hence its allow (the workspace denies it).
#![allow(unsafe_code)]

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    ACL, GetLengthSid, GetTokenInformation, IsValidSid, OBJECT_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::pipe::Sid;

/// The user this process runs as: its token's user SID.
pub(crate) fn own_sid() -> io::Result<Sid> {
    // SAFETY: no arguments; the pseudo-handle it returns needs no closing
    // and stays valid for the life of the process.
    let process = unsafe { GetCurrentProcess() };
    token_user(process)
}

/// The user the process `pid` runs as. Fails when the process is gone or
/// this user may not query it.
pub(super) fn process_sid(pid: u32) -> io::Result<Sid> {
    // SAFETY: plain values in; the returned handle is checked before use.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `OpenProcess` succeeded, so `process` is an open handle that
    // nothing else owns; `OwnedHandle` closes it once.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    token_user(process.as_raw_handle())
}

/// The process at the client end of the server instance `pipe`.
pub(super) fn client_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
    let mut pid = 0;
    // SAFETY: `pipe` is an open pipe handle for the call, borrowed from its
    // owner; `pid` is a valid place for the answer.
    let ok = unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &raw mut pid) };
    known_pid(ok, pid)
}

/// The process at the server end of the client `pipe`.
pub(super) fn server_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
    let mut pid = 0;
    // SAFETY: as in `client_pid`.
    let ok = unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &raw mut pid) };
    known_pid(ok, pid)
}

/// A process id from a call that returned `ok`; never pid 0, which names
/// no user's process.
fn known_pid(ok: i32, pid: u32) -> io::Result<u32> {
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        return Err(io::Error::other("the pipe reports process id 0"));
    }
    Ok(pid)
}

/// The user SID of the process `process` (an open handle with query
/// access, or the current-process pseudo-handle).
fn token_user(process: HANDLE) -> io::Result<Sid> {
    let mut token = ptr::null_mut();
    // SAFETY: `process` is valid for the call (see the callers); `token`
    // is a valid place for the new handle.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `OpenProcessToken` succeeded, so `token` is an open handle
    // that nothing else owns; `OwnedHandle` closes it once.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0u32;
    // SAFETY: a size query, with no buffer and length 0; `needed` is a
    // valid place for the size. It fails by design, so only `needed` counts.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &raw mut needed,
        )
    };
    let size = needed as usize;
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    if size < size_of::<TOKEN_USER>() {
        return Err(io::Error::other(format!(
            "the token's user information is {size} bytes, too small for its record"
        )));
    }
    // `u64`s, so the buffer is aligned for the pointer TOKEN_USER holds.
    let mut buffer = vec![0u64; size.div_ceil(8)];
    // SAFETY: `buffer` is writable for at least `needed` bytes and lives
    // past the call; `needed` is a valid place for the size.
    let filled = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    };
    if filled == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the buffer starts with a TOKEN_USER, and it is
    // aligned and large enough for one (checked above).
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let start = buffer.as_ptr() as usize;
    let within = |at: usize, len: usize| at >= start && at.saturating_add(len) <= start + size;
    if !within(sid as usize, 8) {
        return Err(io::Error::other(
            "the token's user SID lies outside its buffer",
        ));
    }
    // SAFETY: `sid` points inside `buffer`, which is alive, at the SID the
    // call wrote; `IsValidSid` reads only its header and sub-authorities.
    let length = unsafe {
        if IsValidSid(sid) == 0 {
            return Err(io::Error::other("the token's user SID is malformed"));
        }
        GetLengthSid(sid) as usize
    };
    if !within(sid as usize, length) {
        return Err(io::Error::other(
            "the token's user SID lies outside its buffer",
        ));
    }
    // SAFETY: the `length` bytes at `sid` lie inside `buffer` (checked).
    let bytes = unsafe { std::slice::from_raw_parts(sid.cast::<u8>().cast_const(), length) };
    Ok(Sid(bytes.to_vec()))
}

/// A security descriptor the system allocated, freed when dropped. Read
/// only after it is made.
pub(super) struct Descriptor(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is a plain block of memory this value alone owns
// and never changes after it is made; the system reads it from any thread.
unsafe impl Send for Descriptor {}
// SAFETY: as for `Send`: shared references only read it.
unsafe impl Sync for Descriptor {}

impl Descriptor {
    /// A descriptor owned by `owner` whose access list (protected from
    /// inheritance) grants all access to `owner` and to nobody else, and
    /// whose medium mandatory label keeps a process of lower integrity
    /// (a sandboxed one of the same user) from opening it to read or
    /// write.
    pub(super) fn owner_only(owner: &Sid) -> io::Result<Self> {
        let sddl = format!("O:{owner}D:P(A;;FA;;;{owner})S:(ML;;NWNR;;;ME)");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and lives past the call;
        // `descriptor` is a valid place for the result; the size may be
        // left out.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    /// Create an instance of the pipe `name` with `options` and this
    /// descriptor.
    pub(super) fn create(
        &self,
        options: &ServerOptions,
        name: &Path,
    ) -> io::Result<NamedPipeServer> {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        };
        // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES for the call,
        // and the descriptor it points at lives as long as `self`.
        unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast()) }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: `ConvertStringSecurityDescriptorToSecurityDescriptorW`
        // allocated it with `LocalAlloc`, and it is freed only here, once.
        unsafe { LocalFree(self.0) };
    }
}

/// A SID at `sid`, copied; an invalid one is an error.
///
/// # Safety
///
/// `sid` must be null or point at memory that stays readable for the call
/// and, if `IsValidSid` accepts it, holds a whole SID.
unsafe fn copy_sid(sid: PSID) -> io::Result<Sid> {
    // SAFETY: the caller's promise; `IsValidSid` reads only the header and
    // the sub-authorities it counts, and the copy only the length it gives.
    unsafe {
        if sid.is_null() || IsValidSid(sid) == 0 {
            return Err(io::Error::other("the object's SID is missing or malformed"));
        }
        let length = GetLengthSid(sid) as usize;
        Ok(Sid(std::slice::from_raw_parts(
            sid.cast::<u8>().cast_const(),
            length,
        )
        .to_vec()))
    }
}

/// What `GetSecurityInfo` allocated, freed when dropped.
struct Local(PSECURITY_DESCRIPTOR);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: allocated by `GetSecurityInfo` with `LocalAlloc`, freed
        // only here, once.
        unsafe { LocalFree(self.0) };
    }
}

/// The parts `what` names of the kernel object `handle`'s descriptor, as
/// the kernel holds it: the owner, the access list and the system list
/// (labels), each null when not asked for. They point into the returned
/// [`Local`], which must outlive their use.
fn query(
    handle: &impl AsRawHandle,
    what: OBJECT_SECURITY_INFORMATION,
) -> io::Result<(Local, PSID, *mut ACL, *mut ACL)> {
    let (mut owner, mut dacl, mut sacl) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    let mut descriptor = ptr::null_mut();
    // SAFETY: `handle` is open for the call, borrowed from its owner; the
    // out pointers are valid places.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_KERNEL_OBJECT,
            what,
            &raw mut owner,
            ptr::null_mut(),
            &raw mut dacl,
            &raw mut sacl,
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status.cast_signed()));
    }
    Ok((Local(descriptor), owner, dacl, sacl))
}

/// Who owns the kernel object `handle` (a pipe: whoever made it, or the
/// owner its descriptor names). Needs read-control access, which a pipe
/// opened for reading has.
pub(super) fn owner_of(handle: &impl AsRawHandle) -> io::Result<Sid> {
    let (descriptor, owner, _, _) = query(handle, OWNER_SECURITY_INFORMATION)?;
    // SAFETY: `owner` is null or points into `descriptor`, alive until the
    // end of this function.
    let owner = unsafe { copy_sid(owner) };
    drop(descriptor);
    owner
}

/// An access or label entry: its type, mask and SID.
#[cfg(test)]
pub(crate) type Entry = (u8, u32, Sid);

/// A kernel object's descriptor, as the kernel holds it, for tests.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Security {
    pub(crate) owner: Sid,
    /// The access list's entries.
    pub(crate) access: Vec<Entry>,
    /// The mandatory label's entries.
    pub(crate) label: Vec<Entry>,
}

/// The entries of `list`, which points into a live descriptor.
///
/// # Safety
///
/// `list` must be null or point at a valid ACL that stays alive for the
/// call.
#[cfg(test)]
unsafe fn entries(list: *mut ACL) -> io::Result<Vec<Entry>> {
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation,
    };
    if list.is_null() {
        return Ok(Vec::new());
    }
    let mut size = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `list` is a live ACL (the caller's promise); `size` is a
    // valid place of the length given.
    let ok = unsafe {
        GetAclInformation(
            list,
            (&raw mut size).cast(),
            u32::try_from(size_of::<ACL_SIZE_INFORMATION>()).unwrap(),
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut found = Vec::new();
    for index in 0..size.AceCount {
        let mut ace = ptr::null_mut();
        // SAFETY: `index` is below the list's count; `ace` is a valid place.
        if unsafe { GetAce(list, index, &raw mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: an entry of the live list; allowed, denied and label
        // entries all start with a header, a mask and then their SID.
        let entry = unsafe {
            let ace = ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = (&raw const (*ace).SidStart).cast_mut().cast();
            ((*ace).Header.AceType, (*ace).Mask, copy_sid(sid)?)
        };
        found.push(entry);
    }
    Ok(found)
}

/// The [`Security`] of the kernel object `handle`, for tests.
#[cfg(test)]
pub(crate) fn security_of(handle: &impl AsRawHandle) -> io::Result<Security> {
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION};
    let what = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    let (descriptor, owner, dacl, sacl) = query(handle, what)?;
    if dacl.is_null() {
        return Err(io::Error::other("no access list: everyone has access"));
    }
    // SAFETY: each points into `descriptor`, alive until the end of this
    // function, or is null.
    let security = unsafe {
        Security {
            owner: copy_sid(owner)?,
            access: entries(dacl)?,
            label: entries(sacl)?,
        }
    };
    drop(descriptor);
    Ok(security)
}
