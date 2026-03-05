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

/// LLC header length: DSAP(1) + SSAP(1) + Control(1).
pub const LLC_HEADER_LEN: usize = 3;

/// Minimum frame length: Destination(6) + Source(6) + Length(2) + LLC(3).
pub const MIN_FRAME_LEN: usize = 6 + 6 + 2 + LLC_HEADER_LEN; // 17 bytes

/// Maximum IEEE 802.3 length field value. Values above 1500 are EtherType identifiers.
pub const MAX_LLC_LENGTH: usize = 1500;

/// Minimum IEEE 802.3 payload size (excludes 14-byte header and 4-byte FCS).
/// Frames shorter than this must be padded with zeros per 802.3.
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
/// per IEEE 802.3 convention (does not include the 14-byte Ethernet header).
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
    // Pad to minimum IEEE 802.3 frame size (14-byte header + 46-byte payload = 60 bytes)
    let min_frame_size = 14 + MIN_ETHERNET_PAYLOAD;
    if buf.len() < min_frame_size {
        let pad = min_frame_size - buf.len();
        buf.put_bytes(0x00, pad);
    }
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

    // Values > 1500 are EtherType identifiers (e.g. 0x0800 = IPv4), not LLC length.
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
// Linux AF_PACKET transport (Clause 7 / Annex K)
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

    /// BACnet Ethernet transport over raw LLC frames (Clause 7 / Annex K).
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
        fn get_if_index(fd: i32, name: &str) -> Result<i32, Error> {
            let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
            let name_bytes = name.as_bytes();
            if name_bytes.len() >= libc::IFNAMSIZ {
                return Err(Error::Encoding(format!(
                    "interface name too long: {}",
                    name
                )));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    ifr.ifr_name.as_mut_ptr() as *mut u8,
                    name_bytes.len(),
                );
            }
            let ret = unsafe { libc::ioctl(fd, libc::SIOCGIFINDEX as _, &mut ifr) };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            Ok(unsafe { ifr.ifr_ifru.ifru_ifindex })
        }

        /// Query the kernel for the hardware (MAC) address via `SIOCGIFHWADDR`.
        fn get_hw_addr(fd: i32, name: &str) -> Result<[u8; 6], Error> {
            let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
            let name_bytes = name.as_bytes();
            if name_bytes.len() >= libc::IFNAMSIZ {
                return Err(Error::Encoding(format!(
                    "interface name too long: {}",
                    name
                )));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr(),
                    ifr.ifr_name.as_mut_ptr() as *mut u8,
                    name_bytes.len(),
                );
            }
            let ret = unsafe { libc::ioctl(fd, libc::SIOCGIFHWADDR as _, &mut ifr) };
            if ret < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
            let mut mac = [0u8; 6];
            unsafe {
                let data = ifr.ifr_ifru.ifru_hwaddr.sa_data;
                for (i, byte) in mac.iter_mut().enumerate() {
                    *byte = data[i] as u8;
                }
            }
            Ok(mac)
        }
    }

    /// Attach a BPF filter to the raw socket that only accepts BACnet LLC
    /// frames (DSAP=0x82, SSAP=0x82, Control=0x03).
    ///
    /// This is best-effort: if the setsockopt call fails, we log a warning
    /// and continue (software-level filtering in the recv loop still works).
    fn attach_bacnet_bpf_filter(fd: i32) {
        // BPF program:
        //   ldh [12]          ; load length/EtherType field
        //   jgt #1500, drop   ; if > 1500, it's an EtherType frame, drop
        //   ldb [14]          ; DSAP
        //   jneq #0x82, drop
        //   ldb [15]          ; SSAP
        //   jneq #0x82, drop
        //   ldb [16]          ; Control
        //   jneq #0x03, drop
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

        let filter: [SockFilter; 10] = [
            // ldh [12] — load EtherType/Length field
            SockFilter {
                code: BPF_LD | BPF_H | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 12,
            },
            // jgt #1500, drop (offset +7 = instruction 8)
            SockFilter {
                code: BPF_JMP | BPF_JGT | BPF_K,
                jt: 7,
                jf: 0,
                k: 1500,
            },
            // ldb [14] — DSAP
            SockFilter {
                code: BPF_LD | BPF_B | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 14,
            },
            // jneq #0x82, drop (offset +5 = instruction 8)
            SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0,
                jf: 5,
                k: 0x82,
            },
            // ldb [15] — SSAP
            SockFilter {
                code: BPF_LD | BPF_B | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 15,
            },
            // jneq #0x82, drop (offset +3 = instruction 8)
            SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0,
                jf: 3,
                k: 0x82,
            },
            // ldb [16] — Control
            SockFilter {
                code: BPF_LD | BPF_B | BPF_ABS,
                jt: 0,
                jf: 0,
                k: 16,
            },
            // jneq #0x03, drop (offset +1 = instruction 8)
            SockFilter {
                code: BPF_JMP | BPF_JEQ | BPF_K,
                jt: 0,
                jf: 1,
                k: 0x03,
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
            debug!(
                "Attached BPF filter for BACnet LLC frames (DSAP=0x82, SSAP=0x82, Control=0x03)"
            );
        }
    }

    impl TransportPort for EthernetTransport {
        async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
            // Open a raw AF_PACKET socket capturing all EtherType frames.
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
            let owned_fd = unsafe { OwnedFd::from_raw_fd(fd) };

            // Resolve interface index and hardware address.
            self.if_index = Self::get_if_index(owned_fd.as_raw_fd(), &self.interface_name)?;
            self.local_mac = Self::get_hw_addr(owned_fd.as_raw_fd(), &self.interface_name)?;

            debug!(
                interface = %self.interface_name,
                if_index = self.if_index,
                mac = ?self.local_mac,
                "Ethernet transport binding"
            );

            // Attach BPF filter (best-effort) to only accept BACnet LLC frames
            // in the kernel, reducing userspace processing overhead.
            attach_bacnet_bpf_filter(owned_fd.as_raw_fd());

            // Bind to the specific interface.
            let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
            sll.sll_family = libc::AF_PACKET as u16;
            sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
            sll.sll_ifindex = self.if_index;

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

            // Set non-blocking for AsyncFd integration.
            let flags = unsafe { libc::fcntl(owned_fd.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 {
                return Err(Error::Transport(std::io::Error::last_os_error()));
            }
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

            let (tx, rx) = mpsc::channel(256);
            let local_mac = self.local_mac;

            let async_fd = AsyncFd::new(Arc::clone(&owned_fd)).map_err(|e| {
                Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("AsyncFd creation failed: {e}"),
                ))
            })?;

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
                            match decode_ethernet_frame(data) {
                                Ok(frame) => {
                                    // Skip our own frames.
                                    if frame.source == local_mac {
                                        continue;
                                    }
                                    debug!(
                                        src = ?frame.source,
                                        dst = ?frame.destination,
                                        payload_len = frame.payload.len(),
                                        "Ethernet frame received"
                                    );
                                    let _ = tx
                                        .send(ReceivedNpdu {
                                            npdu: frame.payload.clone(),
                                            source_mac: MacAddr::from(frame.source),
                                            reply_tx: None,
                                        })
                                        .await;
                                }
                                Err(_) => {
                                    // Not a BACnet LLC frame — silently skip.
                                    continue;
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            warn!(error = %e, "Ethernet recv error");
                            break;
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

            let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
            sll.sll_family = libc::AF_PACKET as u16;
            sll.sll_ifindex = self.if_index;
            sll.sll_halen = 6;
            sll.sll_addr[..6].copy_from_slice(&dst);

            // Note: The socket is non-blocking. If the kernel send buffer is full, sendto()
            // returns EAGAIN which we surface as a Transport error. This is acceptable for
            // BACnet's low-throughput traffic patterns. Full async writable guards could be
            // added if needed under heavy load.
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
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let dst = ETHERNET_BROADCAST;
        let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let npdu = vec![0x01, 0x00, 0xAA, 0xBB];

        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);

        let decoded = decode_ethernet_frame(&buf).unwrap();
        assert_eq!(decoded.destination, dst);
        assert_eq!(decoded.source, src);
        assert_eq!(decoded.payload, npdu);
    }

    #[test]
    fn llc_header_correct() {
        let dst = [0xFF; 6];
        let src = [0x00; 6];
        let npdu = vec![0xAA];
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
        // LLC at offset 14
        assert_eq!(buf[14], BACNET_LLC_DSAP);
        assert_eq!(buf[15], BACNET_LLC_SSAP);
        assert_eq!(buf[16], LLC_CONTROL_UI);
    }

    #[test]
    fn length_field_correct() {
        let dst = [0xFF; 6];
        let src = [0x00; 6];
        let npdu = vec![0x01, 0x02, 0x03];
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
        let length = u16::from_be_bytes([buf[12], buf[13]]);
        assert_eq!(length as usize, LLC_HEADER_LEN + npdu.len());
    }

    #[test]
    fn rejects_short_frame() {
        assert!(decode_ethernet_frame(&[0; 10]).is_err());
    }

    #[test]
    fn rejects_invalid_llc() {
        let mut buf = vec![0u8; 20];
        buf[14] = 0x00; // wrong DSAP
        buf[15] = 0x82;
        buf[16] = 0x03;
        buf[12] = 0x00;
        buf[13] = 0x04; // length = 4
        assert!(decode_ethernet_frame(&buf).is_err());
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut buf = vec![0u8; MIN_FRAME_LEN];
        buf[14] = BACNET_LLC_DSAP;
        buf[15] = BACNET_LLC_SSAP;
        buf[16] = LLC_CONTROL_UI;
        // Length claims more data than available
        buf[12] = 0x00;
        buf[13] = 0xFF;
        assert!(decode_ethernet_frame(&buf).is_err());
    }

    #[test]
    fn rejects_ethertype_as_length() {
        // A frame with length field = 0x0800 (IPv4 EtherType) should be rejected
        let mut buf = vec![0u8; 20];
        buf[12] = 0x08;
        buf[13] = 0x00; // length = 2048 > 1500
        assert!(decode_ethernet_frame(&buf).is_err());
    }

    #[test]
    fn rejects_length_1501() {
        // Length = 1501 is above the 1500 threshold
        let mut buf = vec![0u8; 20];
        buf[12] = (1501u16 >> 8) as u8;
        buf[13] = (1501u16 & 0xFF) as u8;
        assert!(decode_ethernet_frame(&buf).is_err());
    }

    #[test]
    fn accepts_length_1500() {
        // Length = 1500 is valid (at the boundary)
        // Build a buffer large enough: 14 (header) + 1500 (payload) = 1514
        let mut buf = vec![0u8; 14 + 1500];
        buf[12] = (1500u16 >> 8) as u8;
        buf[13] = (1500u16 & 0xFF) as u8;
        buf[14] = BACNET_LLC_DSAP;
        buf[15] = BACNET_LLC_SSAP;
        buf[16] = LLC_CONTROL_UI;
        let result = decode_ethernet_frame(&buf);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().payload.len(), 1500 - LLC_HEADER_LEN);
    }

    #[test]
    fn encode_pads_small_frame_to_minimum() {
        let dst = [0xFF; 6];
        let src = [0x00; 6];
        // 1-byte NPDU: frame would be 14 + 3 + 1 = 18 bytes without padding
        let npdu = vec![0xAA];
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
        // Must be padded to 60 bytes (14 header + 46 payload)
        assert_eq!(buf.len(), 60);
        // Verify padding is zeros
        for &b in &buf[18..60] {
            assert_eq!(b, 0x00);
        }
    }

    #[test]
    fn encode_does_not_pad_large_frame() {
        let dst = [0xFF; 6];
        let src = [0x00; 6];
        // 50-byte NPDU: frame is 14 + 3 + 50 = 67 bytes, above 60-byte minimum
        let npdu = vec![0xBB; 50];
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
        assert_eq!(buf.len(), 67); // no padding needed
    }

    #[test]
    fn padded_frame_decodes_correctly() {
        let dst = [0xFF; 6];
        let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let npdu = vec![0x01]; // tiny payload
        let mut buf = BytesMut::new();
        encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
        // Should be padded to 60 bytes
        assert_eq!(buf.len(), 60);
        // Decode should extract only the declared payload (not padding)
        let decoded = decode_ethernet_frame(&buf).unwrap();
        assert_eq!(decoded.payload, npdu);
        assert_eq!(decoded.source, src);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ethernet_transport_new() {
        let t = EthernetTransport::new("eth0");
        assert_eq!(t.local_mac(), &[0; 6]); // not started yet
    }
}
