//! BACnet Ethernet transport codec per ASHRAE 135-2020 Clause 7 / Annex K.
//!
//! IEEE 802.3 with LLC framing:
//! - Destination MAC (6) + Source MAC (6) + Length (2) + LLC (3) + NPDU payload
//!
//! Platform: Linux only (AF_PACKET raw sockets). Feature-gated behind `ethernet`.

#[cfg(target_os = "linux")]
use bacnet_types::MacAddr;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
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

/// Whether `mac` is an IEEE 802 group address: six octets whose first has
/// its low (individual/group) bit set. The all-ones broadcast is one, and so
/// is every multicast MAC. A frame sent to one reaches every station that
/// listens for it, so it is a local broadcast, which carries no confirmed
/// request (Clause 6.3, #1493), and no station sends from one.
pub fn is_group_mac(mac: &[u8]) -> bool {
    mac.len() == 6 && mac[0] & 0x01 != 0
}

/// Whether an admitted frame arrived as a group delivery. Ingress admits only
/// this station's own MAC and the all-ones broadcast, so of those only the
/// broadcast is one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_ethernet_group(destination: &[u8; 6]) -> bool {
    *destination == ETHERNET_BROADCAST
}

// Off Linux only the tests reach the send path.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod egress;
mod ingress;

// Local single-link policy: only our unicast or BACnet all-FF broadcast.
// Keep this before all LLC handling; kernel/BPF delivery is not admission.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn accepts_ethernet_destination(data: &[u8], local_mac: &[u8; 6]) -> bool {
    data.get(..6)
        .is_some_and(|destination| destination == local_mac || destination == ETHERNET_BROADCAST)
}

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

/// The refusal of a frame's field at `offset`, `label` naming it with its
/// two numbers.
fn llc_refusal(offset: usize, label: &'static str, first: u64, second: u64) -> Error {
    DecodeError::new(DecodingKind::InvalidEncoding, offset, label)
        .with_values(first, second)
        .into()
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
        return Err(llc_refusal(
            12,
            "length field {} exceeds IEEE 802.3 maximum ({}); likely an EtherType",
            length as u64,
            MAX_LLC_LENGTH as u64,
        ));
    }

    // Length must include at least the LLC header.
    if length < LLC_HEADER_LEN {
        return Err(llc_refusal(
            12,
            "length field {} too small for LLC header ({} bytes)",
            length as u64,
            LLC_HEADER_LEN as u64,
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
        return Err(llc_refusal(
            14,
            "invalid DSAP: expected {:#04X}, got {:#04X}",
            BACNET_LLC_DSAP.into(),
            dsap.into(),
        ));
    }
    if ssap != BACNET_LLC_SSAP {
        return Err(llc_refusal(
            15,
            "invalid SSAP: expected {:#04X}, got {:#04X}",
            BACNET_LLC_SSAP.into(),
            ssap.into(),
        ));
    }
    if control != LLC_CONTROL_UI {
        return Err(llc_refusal(
            16,
            "invalid LLC control: expected {:#04X}, got {:#04X}",
            LLC_CONTROL_UI.into(),
            control.into(),
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
    use bacnet_types::data_link::DataLink;
    use bacnet_types::error::{Lifecycle, Refusal};
    use bacnet_types::sync::RwLockExt;
    use nix::ifaddrs::getifaddrs;
    use nix::net::if_::if_nametoindex;
    use nix::sys::socket::LinkAddr;
    use socket2::{Domain, Protocol, SockFilter, Socket, Type};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;
    use tracing::{debug, warn};

    use super::egress::{ReplyFailures, send_tapped};
    use super::ingress::{npdu_channel_full, receive_frame};
    use crate::port::{ReceivedNpdu, TransportPort, TransportProvenance};
    use crate::stopping::{StopGate, Tasks};
    use crate::sys::packet_socket::AsyncSocket;
    use crate::tap::FrameTap;
    use crate::warn_throttle::WarnThrottle;

    /// Max NPDU size for Ethernet: 1518 (max frame) - 14 (eth header) - 3 (LLC) - 4 (FCS by NIC) = 1497.
    pub const MAX_ETHERNET_NPDU: usize = 1497;

    const _: () = assert!(
        MAX_ETHERNET_NPDU == bacnet_npdu::MAX_NPDU,
        "an Ethernet frame carries the longest NPDU any data link carries"
    );

    /// Refuse an NPDU longer than an Ethernet frame carries as
    /// [`Error::SendTooLong`], as MS/TP and B/IP do (#1833): a router answers
    /// it with reject reason 4, and a client can try a smaller request.
    pub(super) fn fits(npdu: &[u8]) -> Result<(), Error> {
        if npdu.len() > MAX_ETHERNET_NPDU {
            return Err(Error::SendTooLong {
                length: npdu.len(),
                limit: MAX_ETHERNET_NPDU,
            });
        }
        Ok(())
    }

    /// `ETH_P_ALL` in network byte order: the socket's protocol, so it sees
    /// every frame on the interface, whatever its length or EtherType field.
    const ETH_P_ALL_BE: u16 = (libc::ETH_P_ALL as u16).to_be();

    /// BACnet Ethernet transport over raw LLC frames.
    ///
    /// Uses Linux AF_PACKET raw sockets. Requires `CAP_NET_RAW` or root.
    pub struct EthernetTransport {
        interface_name: String,
        local_mac: [u8; 6],
        /// The raw socket, bound to the interface once started. A send holds
        /// the read lock only over its synchronous write, and a stop takes
        /// the socket out under the write lock, so no send holds the socket
        /// once a stop returns (#2617).
        socket: RwLock<Option<Arc<Socket>>>,
        /// The receive task, which a stop through `&self` aborts and joins.
        tasks: Tasks,
        /// One stop at a time (#2617).
        stop_gate: StopGate,
        /// See [`Self::group_source_drops`].
        group_source_drops: Arc<AtomicU64>,
        /// See [`Self::reply_send_failures`].
        reply_failures: Arc<ReplyFailures>,
        /// See [`Self::frame_tap`].
        frame_tap: FrameTap,
    }

    impl EthernetTransport {
        /// Create a new Ethernet transport bound to the given interface name
        /// (e.g. `"eth0"`). Call [`TransportPort::start`] to open the socket.
        pub fn new(interface_name: &str) -> Self {
            Self {
                interface_name: interface_name.to_string(),
                local_mac: [0; 6],
                socket: RwLock::new(None),
                tasks: Tasks::default(),
                stop_gate: StopGate::default(),
                group_source_drops: Arc::default(),
                reply_failures: Arc::default(),
                frame_tap: FrameTap::new(),
            }
        }

        /// This transport's frame tap (#1856): every 802.3 frame it sends
        /// and every one it takes, XID and TEST commands and responses
        /// included, for whoever subscribes. The tap outlives a stop and a
        /// restart; clone it, or subscribe, before handing the transport
        /// over. See [`crate::tap`].
        pub fn frame_tap(&self) -> &FrameTap {
            &self.frame_tap
        }

        /// Frames dropped since this transport was created because their
        /// source MAC is a group address, one with the group bit set (#1492).
        /// No station sends from one, and an answer to such a frame, an XID
        /// or TEST response or a reply to a confirmed request in it, would go
        /// to every station in the group, so the frame is dropped before any
        /// LLC command is answered or the frame is decoded. The total
        /// survives a restart.
        pub fn group_source_drops(&self) -> u64 {
            self.group_source_drops.load(Ordering::Relaxed)
        }

        /// XID and TEST responses (Clause 7.1) that failed to go out since
        /// this transport was created, a short send included (#1918). Each
        /// failure is also logged, as a warning at most once per ten
        /// seconds and at debug otherwise. The total survives a restart.
        pub fn reply_send_failures(&self) -> u64 {
            self.reply_failures.count()
        }
    }

    /// What a send gets while the transport isn't running (#1833): the
    /// error for a stopped transport once `stopped`, else the one before
    /// start. The transport starts again after a stop, so both are
    /// [`Lifecycle::NotRunning`].
    #[cold]
    #[inline(never)]
    fn not_running(stopped: bool) -> Error {
        if stopped {
            return crate::port::stopped();
        }
        Error::state(Lifecycle::NotRunning, "Transport not started")
    }

    /// An error about the interface `name`, of `kind`.
    fn interface_error(
        name: &str,
        kind: std::io::ErrorKind,
        detail: impl std::fmt::Display,
    ) -> Error {
        Error::Transport(std::io::Error::new(
            kind,
            format!("Ethernet interface {name:?}: {detail}"),
        ))
    }

    /// A failed interface lookup (`what`) for `name`. No such device or
    /// address, or a name the kernel can't take, is `NotFound`; anything
    /// else, such as running out of file descriptors, keeps its own kind.
    pub(super) fn lookup_error(name: &str, what: &str, errno: nix::errno::Errno) -> Error {
        use nix::errno::Errno;
        let kind = match errno {
            Errno::ENODEV | Errno::ENXIO | Errno::EINVAL => std::io::ErrorKind::NotFound,
            other => std::io::Error::from(other).kind(),
        };
        interface_error(name, kind, format_args!("{what}: {errno}"))
    }

    /// The AF_PACKET link address of the interface `name` (#1883): its index
    /// from `if_nametoindex`, and its MAC from that index's packet entry in
    /// `getifaddrs`. The entry is also the address the socket binds to. Its
    /// protocol is zero, and a bind with protocol zero keeps the socket's
    /// own, so nix's safe `bind` needs no hand-built `sockaddr_ll`.
    pub(super) fn interface_link(name: &str) -> Result<LinkAddr, Error> {
        use std::io::ErrorKind;
        let index =
            if_nametoindex(name).map_err(|errno| lookup_error(name, "interface index", errno))?;
        let link = getifaddrs()
            .map_err(|errno| lookup_error(name, "interface addresses", errno))?
            .filter_map(|entry| entry.address?.as_link_addr().copied())
            .find(|link| link.ifindex() == index as usize)
            .ok_or_else(|| interface_error(name, ErrorKind::NotFound, "no link-layer address"))?;
        if link.halen() != 6 {
            return Err(interface_error(
                name,
                ErrorKind::InvalidInput,
                format_args!("a {}-octet link address, not a MAC", link.halen()),
            ));
        }
        // Any other protocol would narrow what the bound socket receives.
        if !matches!(link.protocol(), 0 | ETH_P_ALL_BE) {
            return Err(interface_error(
                name,
                ErrorKind::InvalidData,
                format_args!(
                    "link address protocol {:#06x}",
                    u16::from_be(link.protocol())
                ),
            ));
        }
        Ok(link)
    }

    // Classic BPF opcodes (linux/filter.h).
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

    /// The kernel filter: BACnet LLC frames (DSAP 0x82, SSAP 0x82 with or
    /// without the response bit) with any control byte, so UI, XID and TEST
    /// all pass (Clause 7.1):
    ///
    /// ```text
    /// [0] ldh [12]          ; length/EtherType field
    /// [1] jgt #1500, [8]    ; above 1500 it is an EtherType: drop
    /// [2] ldb [14]          ; DSAP
    /// [3] jeq #0x82, [4], [8]
    /// [4] ldb [15]          ; SSAP
    /// [5] and #0xFE         ; mask off the response bit
    /// [6] jeq #0x82, [7], [8]
    /// [7] ret #65535        ; accept
    /// [8] ret #0            ; drop
    /// ```
    const BACNET_LLC_FILTER: [SockFilter; 9] = [
        SockFilter::new(BPF_LD | BPF_H | BPF_ABS, 0, 0, 12),
        SockFilter::new(BPF_JMP | BPF_JGT | BPF_K, 6, 0, 1500),
        SockFilter::new(BPF_LD | BPF_B | BPF_ABS, 0, 0, 14),
        SockFilter::new(BPF_JMP | BPF_JEQ | BPF_K, 0, 4, 0x82),
        SockFilter::new(BPF_LD | BPF_B | BPF_ABS, 0, 0, 15),
        SockFilter::new(BPF_ALU | BPF_AND | BPF_K, 0, 0, 0xFE),
        SockFilter::new(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0x82),
        SockFilter::new(BPF_RET | BPF_K, 0, 0, 65535),
        SockFilter::new(BPF_RET | BPF_K, 0, 0, 0),
    ];

    /// Attach [`BACNET_LLC_FILTER`] to the raw socket. Best-effort: if the
    /// kernel refuses it, warn and go on, since the receive loop filters in
    /// software too.
    fn attach_bacnet_bpf_filter(socket: &Socket) {
        match socket.attach_filter(&BACNET_LLC_FILTER) {
            Ok(()) => debug!(
                "Attached BPF filter for BACnet LLC frames (DSAP=0x82, SSAP=0x82, UI/XID/TEST)"
            ),
            Err(error) => warn!(
                %error,
                "Failed to attach BPF filter for BACnet LLC frames (continuing without kernel filter)"
            ),
        }
    }

    /// Fewer bytes than the kernel charges a receive buffer for any one
    /// queued frame: its `sk_buff` bookkeeping alone is more.
    const MIN_QUEUED_FRAME_COST: usize = 256;

    /// Read and drop every frame already queued on the non-blocking
    /// `socket`, and say how many went. A short buffer is enough: a packet
    /// socket discards what doesn't fit. The reads stop at the most frames
    /// the receive buffer could have held, so a steady stream on the bound
    /// interface can't keep the drain going.
    pub(super) fn drain_queued_frames(socket: &Socket) -> usize {
        let most = socket
            .recv_buffer_size()
            .map_or(4096, |bytes| bytes / MIN_QUEUED_FRAME_COST + 1);
        let mut reader = socket;
        let mut scratch = [0u8; 64];
        let mut dropped = 0;
        while dropped < most {
            match reader.read(&mut scratch) {
                Ok(_) => dropped += 1,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                // WouldBlock: the queue is empty. Any other error is the
                // receive loop's to report.
                Err(_) => break,
            }
        }
        dropped
    }

    impl Drop for EthernetTransport {
        fn drop(&mut self) {
            self.abort();
        }
    }

    impl TransportPort for EthernetTransport {
        fn data_link(&self) -> Option<DataLink> {
            Some(DataLink::Ethernet)
        }

        fn supports_local_nonrouter_number_controls(&self) -> bool {
            true
        }

        /// Abort through `&self` (#2617): the socket comes out, so every
        /// later send fails with [`crate::port::stopped`]'s error, and the
        /// task is aborted. The handle stays, as before: a start after an
        /// abort alone refuses, and a stop still joins the task.
        fn abort(&self) {
            self.stop_gate.abort();
            self.tasks.abort();
            let socket = self.socket.write_locked().take();
            drop(socket);
        }

        async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
            if self.tasks.is_running() {
                return Err(crate::stopping::start_refused(
                    &self.stop_gate,
                    "Ethernet transport already started",
                    "Ethernet transport aborted: stop it before starting again",
                ));
            }

            let link = interface_link(&self.interface_name)?;
            self.local_mac = link.addr().unwrap_or_default();

            debug!(
                interface = %self.interface_name,
                if_index = link.ifindex(),
                mac = ?self.local_mac,
                "Ethernet transport binding"
            );

            // A raw AF_PACKET socket that sees every frame, header included,
            // opened non-blocking (and, by socket2, close-on-exec).
            let socket = Socket::new(
                Domain::PACKET,
                Type::RAW.nonblocking(),
                Some(Protocol::from(i32::from(ETH_P_ALL_BE))),
            )
            .map_err(Error::Transport)?;

            // Attach BPF filter (best-effort) to only accept BACnet LLC frames.
            attach_bacnet_bpf_filter(&socket);

            // Bound to the interface, the socket receives only its frames,
            // and a send goes out on it with no address of its own.
            nix::sys::socket::bind(socket.as_raw_fd(), &link)
                .map_err(|errno| Error::Transport(errno.into()))?;
            // Until the bind, the socket took frames from every interface,
            // so drop whatever queued: none of it reaches classification
            // or draws an XID or TEST reply. Opening with protocol zero and
            // binding with ETH_P_ALL would close the window instead, but
            // that bind needs a hand-built `sockaddr_ll`, which is unsafe.
            let dropped = drain_queued_frames(&socket);
            if dropped > 0 {
                debug!(dropped, "Dropped frames queued before the bind");
            }

            let socket = Arc::new(socket);
            *self.socket.get_mut_locked() = Some(Arc::clone(&socket));

            /// NPDU receive channel capacity for high-throughput raw socket transports.
            const NPDU_CHANNEL_CAPACITY: usize = 256;

            let (tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
            let local_mac = self.local_mac;

            let async_fd = AsyncSocket::register(Arc::clone(&socket)).map_err(|e| {
                Error::Transport(std::io::Error::other(format!(
                    "AsyncFd creation failed: {e}"
                )))
            })?;

            let send_socket = Arc::clone(&socket);
            let group_source_drops = Arc::clone(&self.group_source_drops);
            let reply_failures = Arc::clone(&self.reply_failures);
            let tap = self.frame_tap.clone();
            let recv_task = tokio::spawn(async move {
                let mut recv_buf = vec![0u8; 2048];
                let channel_full = WarnThrottle::default();
                loop {
                    let mut readable = match async_fd.readable().await {
                        Ok(r) => r,
                        Err(e) => {
                            warn!(error = %e, "Ethernet async readable error");
                            break;
                        }
                    };

                    match readable.try_io(|socket| (&**socket.get_ref()).read(&mut recv_buf)) {
                        Ok(Ok(len)) => {
                            // Destination admission, then the group-source
                            // drop (#1492), then the XID and TEST commands
                            // (Clause 7.1), all before a UI frame is decoded;
                            // the tap sees a frame the transport takes first.
                            let Some(frame) = receive_frame(
                                &recv_buf[..len],
                                &local_mac,
                                &tap,
                                &group_source_drops,
                                &reply_failures,
                                |frame| send_socket.send(frame),
                            ) else {
                                continue;
                            };
                            debug!(
                                src = ?frame.source,
                                dst = ?frame.destination,
                                payload_len = frame.payload.len(),
                                "Ethernet frame received"
                            );
                            if tx
                                .try_send(ReceivedNpdu {
                                    direct_response: None,
                                    npdu: frame.payload,
                                    source_mac: MacAddr::from(frame.source),
                                    link_layer_group: is_ethernet_group(&frame.destination),
                                    data_attributes: Vec::new(),
                                    provenance: TransportProvenance::unverified(),
                                    reply_tx: None,
                                })
                                .is_err()
                            {
                                npdu_channel_full(&channel_full);
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

            self.tasks.push(recv_task);
            self.stop_gate.open();
            Ok(rx)
        }

        /// Stop through a shared reference (#2617): the socket comes out of
        /// the send slot, after which every send fails with
        /// [`crate::port::stopped`]'s error, and the receive task is aborted
        /// and joined. The join survives cancellation, so a resumed stop
        /// waits for every receive-task raw-fd reference before reporting
        /// completion. The transport can start again.
        async fn stop(&self) -> Result<(), Error> {
            let Some(stopping) = self.stop_gate.enter().await else {
                return Ok(());
            };
            let socket = self.socket.write_locked().take();
            drop(socket);
            self.tasks.stop().await;
            debug!("Ethernet transport stopped");
            stopping.finish();
            Ok(())
        }

        async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
            // Held over the synchronous send only: there is no await here.
            let socket = self.socket.read_locked();
            let socket = socket
                .as_ref()
                .ok_or_else(|| not_running(self.stop_gate.is_stopped()))?;

            if mac.len() != 6 {
                return Err(Error::InvalidArgument(
                    Refusal::new("Ethernet MAC must be 6 bytes, got {}")
                        .with_value(mac.len() as u64),
                ));
            }

            fits(npdu)?;

            let mut dst = [0u8; 6];
            dst.copy_from_slice(mac);

            let mut buf = BytesMut::with_capacity(MIN_FRAME_LEN + npdu.len());
            encode_ethernet_frame(&mut buf, &dst, &self.local_mac, npdu);

            // The frame carries its own 802.3 header, destination first, and
            // the bound socket sends on its interface: no address needed.
            // A short send is an error (#1918); the frame is tapped just
            // before the send, whether or not it goes (#2519).
            send_tapped(&self.frame_tap, &dst, &buf, |frame| socket.send(frame))
                .map_err(Error::Transport)
        }

        async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
            self.send_unicast(npdu, &ETHERNET_BROADCAST).await
        }

        fn local_receive_apdu_capacity(&self) -> u16 {
            1476
        }

        fn local_mac(&self) -> &[u8] {
            &self.local_mac
        }

        fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
            mac == ETHERNET_BROADCAST
        }

        fn is_group_destination(&self, mac: &[u8]) -> bool {
            is_group_mac(mac)
        }

        fn group_destinations(&self) -> crate::port::GroupDestinations {
            crate::port::GroupDestinations::new(is_group_mac)
        }

        fn egress_apdu_limit(&self) -> u16 {
            1476
        }
    }
}

#[cfg(target_os = "linux")]
pub use transport::EthernetTransport;

#[cfg(test)]
mod tap_tests;
#[cfg(test)]
mod tests;
