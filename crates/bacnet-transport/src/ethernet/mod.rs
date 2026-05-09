//! BACnet Ethernet transport codec per ASHRAE 135-2020 Clause 7 / Annex K.
//!
//! IEEE 802.3 with LLC framing:
//! - Destination MAC (6) + Source MAC (6) + Length (2) + LLC (3) + NPDU payload
//!
//! Platform: Linux only (AF_PACKET raw sockets). Feature-gated behind `ethernet`.

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};

/// LLC DSAP for BACnet (0x82).
pub const BACNET_LLC_DSAP: u8 = 0x82;

/// LLC SSAP for BACnet (0x82).
pub const BACNET_LLC_SSAP: u8 = 0x82;

/// LLC control byte for UI (Unnumbered Information) frames.
pub const LLC_CONTROL_UI: u8 = 0x03;
/// LLC control byte for XID command.
pub const LLC_CONTROL_XID_CMD: u8 = 0xAF;
/// LLC control byte for XID response.
pub const LLC_CONTROL_XID_RSP: u8 = 0xBF;
/// LLC control byte for TEST command.
pub const LLC_CONTROL_TEST_CMD: u8 = 0xE3;
/// LLC control byte for TEST response.
pub const LLC_CONTROL_TEST_RSP: u8 = 0xF3;

/// LLC header length: DSAP(1) + SSAP(1) + Control(1).
pub const LLC_HEADER_LEN: usize = 3;

/// Minimum frame length: Destination(6) + Source(6) + Length(2) + LLC(3).
pub const MIN_FRAME_LEN: usize = 6 + 6 + 2 + LLC_HEADER_LEN; // 17 bytes

/// Maximum IEEE 802.3 length field value. Values above 1500 are EtherType identifiers.
pub const MAX_LLC_LENGTH: usize = 1500;

/// Minimum IEEE 802.3 payload size (excludes 14-byte header and 4-byte FCS).
/// Frames shorter than this must be padded with zeros.
pub const MIN_ETHERNET_PAYLOAD: usize = 46;

/// BACnet broadcast MAC (all 0xFF).
pub const ETHERNET_BROADCAST: [u8; 6] = [0xFF; 6];

/// A decoded BACnet Ethernet LLC frame.
#[derive(Debug, Clone)]
pub struct EthernetFrame {
    /// Destination MAC address (6 bytes).
    pub destination: [u8; 6],
    /// Source MAC address (6 bytes).
    pub source: [u8; 6],
    /// NPDU payload (after LLC header).
    pub payload: Bytes,
}

/// Encode a BACnet Ethernet LLC frame into `buf`.
///
/// Wire format:
/// ```text
/// [dst MAC (6)] [src MAC (6)] [length (2)] [DSAP] [SSAP] [control] [NPDU...]
/// ```
///
/// The length field is the LLC header (3 bytes) plus the NPDU payload length,
/// (does not include the 14-byte Ethernet header).
pub fn encode_ethernet_frame(
    buf: &mut BytesMut,
    destination: &[u8; 6],
    source: &[u8; 6],
    npdu: &[u8],
) {
    let llc_plus_payload = LLC_HEADER_LEN + npdu.len();
    // Reserve at least enough for minimum Ethernet frame (14 header + 46 payload)
    let content_len = 6 + 6 + 2 + LLC_HEADER_LEN + npdu.len();
    let total = content_len.max(14 + MIN_ETHERNET_PAYLOAD);
    buf.reserve(total);
    buf.put_slice(destination);
    buf.put_slice(source);
    buf.put_u16(llc_plus_payload as u16);
    buf.put_u8(BACNET_LLC_DSAP);
    buf.put_u8(BACNET_LLC_SSAP);
    buf.put_u8(LLC_CONTROL_UI);
    buf.put_slice(npdu);
    let min_frame_size = 14 + MIN_ETHERNET_PAYLOAD;
    if buf.len() < min_frame_size {
        let pad = min_frame_size - buf.len();
        buf.put_bytes(0x00, pad);
    }
}

/// Check if a raw frame has BACnet LLC headers (DSAP=0x82, SSAP=0x82).
/// Returns the LLC control byte if the frame is valid, or `None` otherwise.
pub fn check_llc_control(data: &[u8]) -> Option<u8> {
    if data.len() < MIN_FRAME_LEN {
        return None;
    }
    let length = u16::from_be_bytes([data[12], data[13]]) as usize;
    if !(LLC_HEADER_LEN..=MAX_LLC_LENGTH).contains(&length) {
        return None;
    }
    if data[14] == BACNET_LLC_DSAP && data[15] == BACNET_LLC_SSAP {
        Some(data[16])
    } else {
        None
    }
}

/// Build an XID response frame (Clause 7.1).
///
/// Swaps src/dest, sets control to XID response (0xBF), SSAP response bit set (0x83).
pub fn build_xid_response(local_mac: &[u8; 6], remote_mac: &[u8; 6]) -> Vec<u8> {
    // XID format: dest(6) + src(6) + length(2) + DSAP(1) + SSAP(1) + control(1) + XID info(3)
    // XID info: format identifier (0x81), types (0x01), window size (0x01)
    let llc_payload = [0x81u8, 0x01, 0x01]; // IEEE 802.2 XID format
    let length = LLC_HEADER_LEN + llc_payload.len();
    let mut buf = Vec::with_capacity(60);
    buf.extend_from_slice(remote_mac);
    buf.extend_from_slice(local_mac);
    buf.extend_from_slice(&(length as u16).to_be_bytes());
    buf.push(BACNET_LLC_DSAP);
    buf.push(BACNET_LLC_SSAP | 0x01); // response bit set (0x83)
    buf.push(LLC_CONTROL_XID_RSP);
    buf.extend_from_slice(&llc_payload);
    // Pad to minimum frame size
    let min_size = 14 + MIN_ETHERNET_PAYLOAD;
    if buf.len() < min_size {
        buf.resize(min_size, 0x00);
    }
    buf
}

/// Build a TEST response frame (Clause 7.1).
///
/// Swaps src/dest, echoes the test data back with control set to TEST response (0xF3).
pub fn build_test_response(local_mac: &[u8; 6], remote_mac: &[u8; 6], test_data: &[u8]) -> Vec<u8> {
    let length = LLC_HEADER_LEN + test_data.len();
    let mut buf = Vec::with_capacity(14 + length + MIN_ETHERNET_PAYLOAD);
    buf.extend_from_slice(remote_mac);
    buf.extend_from_slice(local_mac);
    buf.extend_from_slice(&(length as u16).to_be_bytes());
    buf.push(BACNET_LLC_DSAP);
    buf.push(BACNET_LLC_SSAP | 0x01); // response bit set (0x83)
    buf.push(LLC_CONTROL_TEST_RSP);
    buf.extend_from_slice(test_data);
    // Pad to minimum frame size
    let min_size = 14 + MIN_ETHERNET_PAYLOAD;
    if buf.len() < min_size {
        buf.resize(min_size, 0x00);
    }
    buf
}

/// Decode a BACnet Ethernet LLC frame from raw bytes.
///
/// Validates minimum length, LLC header fields (DSAP, SSAP, control),
/// and that the length field does not exceed the available data.
pub fn decode_ethernet_frame(data: &[u8]) -> Result<EthernetFrame, Error> {
    if data.len() < MIN_FRAME_LEN {
        return Err(Error::buffer_too_short(MIN_FRAME_LEN, data.len()));
    }

    let mut destination = [0u8; 6];
    destination.copy_from_slice(&data[0..6]);

    let mut source = [0u8; 6];
    source.copy_from_slice(&data[6..12]);

    let length = u16::from_be_bytes([data[12], data[13]]) as usize;

    if length > MAX_LLC_LENGTH {
        return Err(Error::decoding(
            12,
            format!(
                "length field {length} exceeds IEEE 802.3 maximum ({MAX_LLC_LENGTH}); likely an EtherType"
            ),
        ));
    }

    // Length must include at least the LLC header.
    if length < LLC_HEADER_LEN {
        return Err(Error::decoding(
            12,
            format!("length field {length} too small for LLC header ({LLC_HEADER_LEN} bytes)"),
        ));
    }

    // Check that the frame contains enough bytes for the declared length.
    let end = 14 + length; // 14 = dst(6) + src(6) + length(2)
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }

    // Validate LLC header.
    let dsap = data[14];
    let ssap = data[15];
    let control = data[16];

    if dsap != BACNET_LLC_DSAP {
        return Err(Error::decoding(
            14,
            format!("invalid DSAP: expected 0x{BACNET_LLC_DSAP:02X}, got 0x{dsap:02X}"),
        ));
    }
    if ssap != BACNET_LLC_SSAP {
        return Err(Error::decoding(
            15,
            format!("invalid SSAP: expected 0x{BACNET_LLC_SSAP:02X}, got 0x{ssap:02X}"),
        ));
    }
    if control != LLC_CONTROL_UI {
        return Err(Error::decoding(
            16,
            format!("invalid LLC control: expected 0x{LLC_CONTROL_UI:02X}, got 0x{control:02X}"),
        ));
    }

    let payload = Bytes::copy_from_slice(&data[17..end]);

    Ok(EthernetFrame {
        destination,
        source,
        payload,
    })
}

// ---------------------------------------------------------------------------
// Linux AF_PACKET transport
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod transport {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::Arc;
    use tokio::io::unix::AsyncFd;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tracing::{debug, warn};

    use crate::port::{ReceivedNpdu, TransportPort};

    /// Max NPDU size for Ethernet: 1518 (max frame) - 14 (eth header) - 3 (LLC) - 4 (FCS by NIC) = 1497.
    pub const MAX_ETHERNET_NPDU: usize = 1497;

    /// BACnet Ethernet transport over raw LLC frames.
    ///
    /// Uses Linux AF_PACKET raw sockets. Requires `CAP_NET_RAW` or root.
    pub struct EthernetTransport {
        interface_name: String,
        local_mac: [u8; 6],
        raw_fd: Option<Arc<OwnedFd>>,
        if_index: i32,
        recv_task: Option<JoinHandle<()>>,
    }

    impl EthernetTransport {
        /// Create a new Ethernet transport bound to the given interface name
        /// (e.g. `"eth0"`). Call [`TransportPort::start`] to open the socket.
        pub fn new(interface_name: &str) -> Self {
            Self {
                interface_name: interface_name.to_string(),
                local_mac: [0; 6],
                raw_fd: None,
                if_index: 0,
                recv_task: None,
            }
        }

        /// Query the kernel for the interface index via `SIOCGIFINDEX`.
        #[allow(unsafe_code)]
        fn get_if_index(fd: i32, name: &str) -> Result<i32, Error> {
            // SAFETY: `libc::ifreq` is a C plain-old-data struct; zero-init produces
            // a valid all-zero `ifreq` (cleared name, zero union members).
            let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
            let name_bytes = name.as_bytes();
            if name_bytes.len() >= libc::IFNAMSIZ {
                return Err(Error::Encoding(format!(
                    "interface name too long: {}",
                    name
                )));
            }
            // SAFETY: bounds checked above (`name_bytes.len() < IFNAMSIZ`); src and dst
            // are non-overlapping (`name_bytes` borrows the caller's str, `ifr` is local).
            unsafe {
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    ifr.ifr_name.as_mut_ptr() as *mut u8,
                    name_bytes.len(),
                );
            }
            // SAFETY: `fd` is a valid open AF_PACKET socket owned by the caller; `ifr`
            // is a properly-initialized `ifreq` borrowed mutably for the duration of the call.
            let ret = unsafe { libc::ioctl(fd, libc::SIOCGIFINDEX as _, &mut ifr) };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            // SAFETY: kernel populated `ifr_ifru.ifru_ifindex` on the successful ioctl above
            // (selected union variant matches the SIOCGIFINDEX request).
            Ok(unsafe { ifr.ifr_ifru.ifru_ifindex })
        }

        /// Query the kernel for the hardware (MAC) address via `SIOCGIFHWADDR`.
        #[allow(unsafe_code)]
        fn get_hw_addr(fd: i32, name: &str) -> Result<[u8; 6], Error> {
            // SAFETY: `libc::ifreq` is C plain-old-data; zero-init is a valid value.
            let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
            let name_bytes = name.as_bytes();
            if name_bytes.len() >= libc::IFNAMSIZ {
                return Err(Error::Encoding(format!(
                    "interface name too long: {}",
                    name
                )));
            }
            // SAFETY: bounds checked above; src/dst are non-overlapping (separate allocations).
            unsafe {
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    ifr.ifr_name.as_mut_ptr() as *mut u8,
                    name_bytes.len(),
                );
            }
            // SAFETY: `fd` is a valid open AF_PACKET socket; `ifr` is properly initialized.
            let ret = unsafe { libc::ioctl(fd, libc::SIOCGIFHWADDR as _, &mut ifr) };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            let mut mac = [0u8; 6];
            // SAFETY: kernel populated the `ifru_hwaddr` union variant on the successful
            // SIOCGIFHWADDR ioctl above; first 6 bytes of `sa_data` are the IEEE MAC.
            unsafe {
                let data = ifr.ifr_ifru.ifru_hwaddr.sa_data;
                for (i, byte) in mac.iter_mut().enumerate() {
                    *byte = data[i] as u8;
                }
            }
            Ok(mac)
        }
    }

    impl EthernetTransport {
        /// Send a raw pre-built Ethernet frame via the AF_PACKET socket.
        /// Used for LLC command responses (XID, TEST) which bypass the normal
        /// BACnet NPDU encode path.
        #[allow(unsafe_code)]
        fn raw_sendto(
            fd: i32,
            if_index: i32,
            dest_mac: &[u8; 6],
            frame: &[u8],
        ) -> Result<(), std::io::Error> {
            // SAFETY: `sockaddr_ll` is C POD; zero-init is a valid value before fields are set.
            let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
            sll.sll_family = libc::AF_PACKET as u16;
            sll.sll_ifindex = if_index;
            sll.sll_halen = 6;
            sll.sll_addr[..6].copy_from_slice(dest_mac);
            // SAFETY: `fd` is a valid AF_PACKET raw socket owned by the caller; `frame.as_ptr()`
            // is valid for `frame.len()` bytes; `sll` is fully initialized above and valid for
            // `size_of::<sockaddr_ll>()` bytes.
            let ret = unsafe {
                libc::sendto(
                    fd,
                    frame.as_ptr() as *const libc::c_void,
                    frame.len(),
                    0,
                    &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                )
            };
            if ret < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    /// Attach a BPF filter to the raw socket that accepts BACnet LLC
    /// frames (DSAP=0x82, SSAP=0x82) with any control byte (UI, XID, TEST).
    ///
    /// This is best-effort: if the setsockopt call fails, we log a warning
    /// and continue (software-level filtering in the recv loop still works).
    #[allow(unsafe_code)]
    fn attach_bacnet_bpf_filter(fd: i32) {
        // BPF program (no control byte check — accept UI, XID, and TEST per Clause 7.1):
        //   ldh [12]          ; load length/EtherType field
        //   jgt #1500, drop   ; if > 1500, it's an EtherType frame, drop
        //   ldb [14]          ; DSAP
        //   jneq #0x82, drop
        //   ldb [15]          ; SSAP (accept 0x82 and 0x83 response bit)
        //   and #0xFE         ; mask off response bit
        //   jneq #0x82, drop
        //   ret #65535        ; accept
        //   drop: ret #0      ; reject
        #[repr(C)]
        #[derive(Copy, Clone)]
        struct SockFilter {
            code: u16,
            jt: u8,
            jf: u8,
            k: u32,
        }

        #[repr(C)]
        struct SockFprog {
            len: u16,
            filter: *const SockFilter,
        }

        // BPF opcodes
        const BPF_LD: u16 = 0x00;
        const BPF_H: u16 = 0x08; // half-word (2 bytes)
        const BPF_B: u16 = 0x10; // byte
        const BPF_ABS: u16 = 0x20; // absolute offset
        const BPF_JMP: u16 = 0x05;
        const BPF_JGT: u16 = 0x20;
        const BPF_JEQ: u16 = 0x10;
        const BPF_RET: u16 = 0x06;
        const BPF_K: u16 = 0x00;

        const BPF_ALU: u16 = 0x04;
        const BPF_AND: u16 = 0x50;

        // 9 instructions: 7 filter + accept + drop
        let filter: [SockFilter; 9] = [
            // [0] ldh [12] — load EtherType/Length field
            SockFilter {
                code: BPF_LD | BPF_H | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 12,
            },
            // [1] jgt #1500, drop(8) — skip 6 forward on true
            SockFilter {
                code: BPF_JMP | BPF_JGT | BPF_K,
                jt: 6,
                jf: 0,
                k: 1500,
            },
            // [2] ldb [14] — DSAP
            SockFilter {
                code: BPF_LD | BPF_B | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 14,
            },
            // [3] jeq #0x82, continue, drop(8) — skip 4 forward on false
            SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0,
                jf: 4,
                k: 0x82,
            },
            // [4] ldb [15] — SSAP
            SockFilter {
                code: BPF_LD | BPF_B | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 15,
            },
            // [5] and #0xFE — mask off LLC response bit
            SockFilter {
                code: BPF_ALU | BPF_AND | BPF_K,
                jt: 0,
                jf: 0,
                k: 0xFE,
            },
            // [6] jeq #0x82, accept(7), drop(8) — skip 0/1
            SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0,
                jf: 1,
                k: 0x82,
            },
            // ret #65535 — accept
            SockFilter {
                code: BPF_RET | BPF_K,
                jt: 0,
                jf: 0,
                k: 65535,
            },
            // drop: ret #0 — reject
            SockFilter {
                code: BPF_RET | BPF_K,
                jt: 0,
                jf: 0,
                k: 0,
            },
        ];

        let prog = SockFprog {
            len: filter.len() as u16,
            filter: filter.as_ptr(),
        };

        // SAFETY: `fd` is the caller's open AF_PACKET socket; `prog` references the local
        // `filter` array which outlives the call (kernel copies the program by value).
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ATTACH_FILTER,
                &prog as *const SockFprog as *const libc::c_void,
                std::mem::size_of::<SockFprog>() as libc::socklen_t,
            )
        };
        if ret < 0 {
            warn!(
                error = %std::io::Error::last_os_error(),
                "Failed to attach BPF filter for BACnet LLC frames (continuing without kernel filter)"
            );
        } else {
            debug!("Attached BPF filter for BACnet LLC frames (DSAP=0x82, SSAP=0x82, UI/XID/TEST)");
        }
    }

    impl TransportPort for EthernetTransport {
        #[allow(unsafe_code)]
        async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
            if self.recv_task.is_some() {
                return Err(Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "Ethernet transport already started",
                )));
            }

            // Open a raw AF_PACKET socket capturing all EtherType frames.
            // SAFETY: pure syscall — no Rust-side memory invariants. Returned fd is
            // checked for `< 0` and wrapped in `OwnedFd` below before any other use.
            let fd = unsafe {
                libc::socket(
                    libc::AF_PACKET,
                    libc::SOCK_RAW,
                    (libc::ETH_P_ALL as u16).to_be() as i32,
                )
            };
            if fd < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            // SAFETY: `fd >= 0` (checked above); we are the sole owner of this freshly
            // opened socket and transfer ownership to `OwnedFd` so it is closed on drop.
            let owned_fd = unsafe { OwnedFd::from_raw_fd(fd) };

            self.if_index = Self::get_if_index(owned_fd.as_raw_fd(), &self.interface_name)?;
            self.local_mac = Self::get_hw_addr(owned_fd.as_raw_fd(), &self.interface_name)?;

            debug!(
                interface = %self.interface_name,
                if_index = self.if_index,
                mac = ?self.local_mac,
                "Ethernet transport binding"
            );

            // Attach BPF filter (best-effort) to only accept BACnet LLC frames.
            attach_bacnet_bpf_filter(owned_fd.as_raw_fd());

            // SAFETY: `sockaddr_ll` is C POD; zero-init is valid before fields are set.
            let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
            sll.sll_family = libc::AF_PACKET as u16;
            sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
            sll.sll_ifindex = self.if_index;

            // SAFETY: `owned_fd` is a valid AF_PACKET socket we just opened; `sll` is
            // fully initialized above and valid for `size_of::<sockaddr_ll>()` bytes.
            let ret = unsafe {
                libc::bind(
                    owned_fd.as_raw_fd(),
                    &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                )
            };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }

            // SAFETY: `owned_fd` is a valid open socket; `F_GETFL` reads flags and has no
            // pointer arguments to validate.
            let flags = unsafe { libc::fcntl(owned_fd.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            // SAFETY: `owned_fd` is a valid open socket; `F_SETFL` takes an int flag.
            let ret = unsafe {
                libc::fcntl(
                    owned_fd.as_raw_fd(),
                    libc::F_SETFL,
                    flags | libc::O_NONBLOCK,
                )
            };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }

            let owned_fd = Arc::new(owned_fd);
            self.raw_fd = Some(Arc::clone(&owned_fd));

            /// NPDU receive channel capacity for high-throughput raw socket transports.
            const NPDU_CHANNEL_CAPACITY: usize = 256;

            let (tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
            let local_mac = self.local_mac;

            let async_fd = AsyncFd::new(Arc::clone(&owned_fd)).map_err(|e| {
                Error::Transport(std::io::Error::other(format!(
                    "AsyncFd creation failed: {e}"
                )))
            })?;

            let send_fd = Arc::clone(&owned_fd);
            let if_index = self.if_index;
            let recv_task = tokio::spawn(async move {
                let mut recv_buf = vec![0u8; 2048];
                loop {
                    let mut readable = match async_fd.readable().await {
                        Ok(r) => r,
                        Err(e) => {
                            warn!(error = %e, "Ethernet async readable error");
                            break;
                        }
                    };

                    match readable.try_io(|fd| {
                        // SAFETY: `fd` borrows the live `OwnedFd` from `AsyncFd` (still open
                        // since the task holds `Arc<OwnedFd>`); `recv_buf` is a heap buffer of
                        // `len()` bytes valid for write for the duration of the call.
                        #[allow(unsafe_code)]
                        let n = unsafe {
                            libc::recv(
                                fd.get_ref().as_raw_fd(),
                                recv_buf.as_mut_ptr() as *mut libc::c_void,
                                recv_buf.len(),
                                0,
                            )
                        };
                        if n < 0 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(n as usize)
                        }
                    }) {
                        Ok(Ok(len)) => {
                            let data = &recv_buf[..len];

                            // Handle XID/TEST commands before UI decode (Clause 7.1)
                            if let Some(control) = check_llc_control(data) {
                                if data.len() >= 12 && data[6..12] != local_mac {
                                    let mut src_mac = [0u8; 6];
                                    src_mac.copy_from_slice(&data[6..12]);
                                    match control {
                                        LLC_CONTROL_XID_CMD => {
                                            debug!(src = ?src_mac, "XID command, sending response");
                                            let resp = build_xid_response(&local_mac, &src_mac);
                                            let _ = EthernetTransport::raw_sendto(
                                                send_fd.as_raw_fd(),
                                                if_index,
                                                &src_mac,
                                                &resp,
                                            );
                                            continue;
                                        }
                                        LLC_CONTROL_TEST_CMD => {
                                            // Echo back the test data (bytes after LLC header)
                                            let test_data =
                                                if data.len() > 17 { &data[17..] } else { &[] };
                                            debug!(src = ?src_mac, len = test_data.len(), "TEST command, sending response");
                                            let resp = build_test_response(
                                                &local_mac, &src_mac, test_data,
                                            );
                                            let _ = EthernetTransport::raw_sendto(
                                                send_fd.as_raw_fd(),
                                                if_index,
                                                &src_mac,
                                                &resp,
                                            );
                                            continue;
                                        }
                                        _ => {} // UI and others fall through to decode
                                    }
                                }
                            }

                            match decode_ethernet_frame(data) {
                                Ok(frame) => {
                                    if frame.source == local_mac {
                                        continue;
                                    }
                                    debug!(
                                        src = ?frame.source,
                                        dst = ?frame.destination,
                                        payload_len = frame.payload.len(),
                                        "Ethernet frame received"
                                    );
                                    if tx
                                        .try_send(ReceivedNpdu {
                                            npdu: frame.payload.clone(),
                                            source_mac: MacAddr::from(frame.source),
                                            reply_tx: None,
                                        })
                                        .is_err()
                                    {
                                        warn!(
                                            "Ethernet: NPDU channel full, dropping incoming frame"
                                        );
                                    }
                                }
                                Err(_) => {
                                    continue;
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            let errno = e.raw_os_error().unwrap_or(0);
                            match errno {
                                libc::EAGAIN | libc::EINTR | libc::ENOBUFS | libc::ENOMEM => {
                                    debug!(error = %e, "Ethernet: transient recv error");
                                    continue;
                                }
                                _ => {
                                    tracing::error!(error = %e, "Ethernet: fatal recv error");
                                    break;
                                }
                            }
                        }
                        Err(_would_block) => continue,
                    }
                }
            });

            self.recv_task = Some(recv_task);
            Ok(rx)
        }

        async fn stop(&mut self) -> Result<(), Error> {
            if let Some(task) = self.recv_task.take() {
                task.abort();
                let _ = task.await;
            }
            self.raw_fd = None;
            debug!("Ethernet transport stopped");
            Ok(())
        }

        #[allow(unsafe_code)]
        async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
            let fd = self.raw_fd.as_ref().ok_or_else(|| {
                Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "Transport not started",
                ))
            })?;

            if mac.len() != 6 {
                return Err(Error::Encoding(format!(
                    "Ethernet MAC must be 6 bytes, got {}",
                    mac.len()
                )));
            }

            if npdu.len() > MAX_ETHERNET_NPDU {
                return Err(Error::Encoding(format!(
                    "NPDU too large for Ethernet: {} bytes (max {})",
                    npdu.len(),
                    MAX_ETHERNET_NPDU
                )));
            }

            let mut dst = [0u8; 6];
            dst.copy_from_slice(mac);

            let mut buf = BytesMut::with_capacity(MIN_FRAME_LEN + npdu.len());
            encode_ethernet_frame(&mut buf, &dst, &self.local_mac, npdu);

            // SAFETY: `sockaddr_ll` is C POD; zero-init is valid before fields are set.
            let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
            sll.sll_family = libc::AF_PACKET as u16;
            sll.sll_ifindex = self.if_index;
            sll.sll_halen = 6;
            sll.sll_addr[..6].copy_from_slice(&dst);

            // SAFETY: `fd` is a live `OwnedFd` borrowed via `Arc`; `buf.as_ptr()` is valid
            // for `buf.len()` bytes; `sll` is fully initialized; size matches `socklen_t`.
            let ret = unsafe {
                libc::sendto(
                    fd.as_raw_fd(),
                    buf.as_ptr() as *const libc::c_void,
                    buf.len(),
                    0,
                    &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                )
            };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            Ok(())
        }

        async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
            self.send_unicast(npdu, &ETHERNET_BROADCAST).await
        }

        fn local_mac(&self) -> &[u8] {
            &self.local_mac
        }

        fn max_apdu_length(&self) -> u16 {
            1476
        }
    }
}

#[cfg(target_os = "linux")]
pub use transport::EthernetTransport;

#[cfg(test)]
mod tests;
