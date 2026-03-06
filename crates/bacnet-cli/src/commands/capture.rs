//! BACnet packet capture command — live capture or pcap file reading.
//!
//! This entire module is gated behind `#[cfg(feature = "pcap")]`.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::decode;
use crate::output::OutputFormat;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for the capture command.
pub struct CaptureOpts {
    pub read: Option<PathBuf>,
    pub save: Option<PathBuf>,
    pub quiet: bool,
    pub decode: bool,
    pub device: Option<String>,
    pub interface_ip: Ipv4Addr,
    pub filter: Option<String>,
    pub count: Option<u64>,
    pub snaplen: u32,
    pub format: OutputFormat,
}

// ---------------------------------------------------------------------------
// Capture source abstraction
// ---------------------------------------------------------------------------

/// Error type for capture source operations.
#[derive(Debug)]
enum PcapError {
    Timeout,
    Eof,
    Other(String),
}

impl std::fmt::Display for PcapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PcapError::Timeout => write!(f, "timeout"),
            PcapError::Eof => write!(f, "end of file"),
            PcapError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// A captured packet with owned data (avoids lifetime issues with pcap borrows).
struct CapturedPacket {
    header: pcap::PacketHeader,
    data: Vec<u8>,
}

/// Unified capture source trait over live and offline pcap handles.
trait CaptureSource {
    fn next_packet(&mut self) -> Result<CapturedPacket, PcapError>;
    fn set_filter(&mut self, filter: &str) -> Result<(), Box<dyn std::error::Error>>;
    fn datalink(&self) -> pcap::Linktype;
    fn savefile(
        &self,
        path: &std::path::Path,
    ) -> Result<pcap::Savefile, Box<dyn std::error::Error>>;
}

fn map_pcap_error(e: pcap::Error) -> PcapError {
    match e {
        pcap::Error::TimeoutExpired => PcapError::Timeout,
        pcap::Error::NoMorePackets => PcapError::Eof,
        other => PcapError::Other(other.to_string()),
    }
}

struct LiveCapture(pcap::Capture<pcap::Active>);

impl CaptureSource for LiveCapture {
    fn next_packet(&mut self) -> Result<CapturedPacket, PcapError> {
        let packet = self.0.next_packet().map_err(map_pcap_error)?;
        Ok(CapturedPacket {
            header: *packet.header,
            data: packet.data.to_vec(),
        })
    }

    fn set_filter(&mut self, filter: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.0.filter(filter, true)?;
        Ok(())
    }

    fn datalink(&self) -> pcap::Linktype {
        self.0.get_datalink()
    }

    fn savefile(
        &self,
        path: &std::path::Path,
    ) -> Result<pcap::Savefile, Box<dyn std::error::Error>> {
        Ok(self.0.savefile(path)?)
    }
}

struct OfflineCapture(pcap::Capture<pcap::Offline>);

impl CaptureSource for OfflineCapture {
    fn next_packet(&mut self) -> Result<CapturedPacket, PcapError> {
        let packet = self.0.next_packet().map_err(map_pcap_error)?;
        Ok(CapturedPacket {
            header: *packet.header,
            data: packet.data.to_vec(),
        })
    }

    fn set_filter(&mut self, filter: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.0.filter(filter, true)?;
        Ok(())
    }

    fn datalink(&self) -> pcap::Linktype {
        self.0.get_datalink()
    }

    fn savefile(
        &self,
        path: &std::path::Path,
    ) -> Result<pcap::Savefile, Box<dyn std::error::Error>> {
        Ok(self.0.savefile(path)?)
    }
}

// ---------------------------------------------------------------------------
// JSON output struct
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct PacketJson {
    timestamp: String,
    src: String,
    dst: String,
    bvlc: String,
    service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Build a BPF filter string. Defaults to "udp port 47808"; if the user
/// supplies an additional expression it is appended with `and`.
fn build_filter(user_filter: &Option<String>) -> String {
    let base = "udp port 47808";
    match user_filter {
        Some(expr) => format!("{base} and ({expr})"),
        None => base.to_string(),
    }
}

/// Resolve a pcap device from a name, interface IP, or system default.
fn resolve_device(
    device_name: &Option<String>,
    interface_ip: Ipv4Addr,
) -> Result<pcap::Device, String> {
    if let Some(name) = device_name {
        let devices = pcap::Device::list().map_err(|e| format!("cannot list devices: {e}"))?;
        devices
            .into_iter()
            .find(|d| d.name == *name)
            .ok_or_else(|| format!("device '{name}' not found"))
    } else if !interface_ip.is_unspecified() {
        let devices = pcap::Device::list().map_err(|e| format!("cannot list devices: {e}"))?;
        let target = std::net::IpAddr::V4(interface_ip);
        devices
            .into_iter()
            .find(|d| d.addresses.iter().any(|a| a.addr == target))
            .ok_or_else(|| format!("no device found for IP {interface_ip}"))
    } else {
        pcap::Device::lookup()
            .map_err(|e| format!("cannot find default device: {e}"))?
            .ok_or_else(|| "no default capture device found".to_string())
    }
}

/// Format a `libc::timeval` as `HH:MM:SS.mmm`.
fn format_timestamp(ts: &libc::timeval) -> String {
    let total_secs = ts.tv_sec as u64;
    let day_secs = total_secs % 86400;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;
    let millis = ts.tv_usec / 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Find the start of the IP header within a raw captured frame.
fn ip_offset(datalink: pcap::Linktype) -> Option<usize> {
    if datalink == pcap::Linktype::ETHERNET {
        Some(14) // 14-byte Ethernet header
    } else if datalink == pcap::Linktype(12) {
        // DLT_RAW — starts at IP
        Some(0)
    } else if datalink == pcap::Linktype::NULL {
        Some(4) // BSD loopback: 4-byte header
    } else {
        None
    }
}

/// Extract source and destination IP:port strings from a raw captured frame.
fn extract_ip_addrs(raw: &[u8], datalink: pcap::Linktype) -> Option<(String, String)> {
    let offset = ip_offset(datalink)?;
    if raw.len() < offset + 20 {
        return None;
    }
    let ip = &raw[offset..];
    let ihl = ((ip[0] & 0x0F) as usize) * 4;
    if ip.len() < ihl + 8 {
        return None;
    }
    let src_ip = format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]);
    let dst_ip = format!("{}.{}.{}.{}", ip[16], ip[17], ip[18], ip[19]);
    let udp = &ip[ihl..];
    let src_port = u16::from_be_bytes([udp[0], udp[1]]);
    let dst_port = u16::from_be_bytes([udp[2], udp[3]]);
    Some((
        format!("{src_ip}:{src_port}"),
        format!("{dst_ip}:{dst_port}"),
    ))
}

/// Extract the UDP payload (BACnet data) from a raw captured frame.
fn extract_bacnet_payload(raw: &[u8], datalink: pcap::Linktype) -> Option<&[u8]> {
    let offset = ip_offset(datalink)?;
    if raw.len() < offset + 20 {
        return None;
    }
    let ip = &raw[offset..];
    let ihl = ((ip[0] & 0x0F) as usize) * 4;
    if ip.len() < ihl + 8 {
        return None;
    }
    let udp = &ip[ihl..];
    let udp_len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if udp_len < 8 || udp.len() < udp_len {
        return None;
    }
    Some(&udp[8..udp_len])
}

/// Install a Ctrl-C handler that clears the running flag.
fn ctrlc_flag(flag: &Arc<AtomicBool>) {
    let f = flag.clone();
    let _ = ctrlc::set_handler(move || {
        f.store(false, Ordering::Relaxed);
    });
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run the capture command.
pub fn run_capture(opts: CaptureOpts) -> Result<(), Box<dyn std::error::Error>> {
    // Validate options
    if opts.read.is_some() && opts.device.is_some() {
        return Err("--read and --device are mutually exclusive".into());
    }
    if opts.quiet && opts.save.is_none() {
        return Err("--quiet requires --save".into());
    }

    let filter_str = build_filter(&opts.filter);

    // Open capture source
    let mut source: Box<dyn CaptureSource> = if let Some(ref path) = opts.read {
        let cap = pcap::Capture::from_file(path)?;
        Box::new(OfflineCapture(cap))
    } else {
        let device = resolve_device(&opts.device, opts.interface_ip)?;
        let cap = pcap::Capture::from_device(device)?
            .snaplen(opts.snaplen as i32)
            .timeout(1000)
            .open()?;
        Box::new(LiveCapture(cap))
    };

    // Apply BPF filter
    source.set_filter(&filter_str)?;

    // Open savefile if requested
    let mut savefile = match &opts.save {
        Some(path) => Some(source.savefile(path)?),
        None => None,
    };

    let datalink = source.datalink();

    // Ctrl-C handler
    let running = Arc::new(AtomicBool::new(true));
    ctrlc_flag(&running);

    let mut packet_count: u64 = 0;

    loop {
        // Check running flag and count limit
        if !running.load(Ordering::Relaxed) {
            break;
        }
        if let Some(limit) = opts.count {
            if packet_count >= limit {
                break;
            }
        }

        // Get next packet
        let pkt = match source.next_packet() {
            Ok(p) => p,
            Err(PcapError::Timeout) => continue,
            Err(PcapError::Eof) => break,
            Err(PcapError::Other(e)) => return Err(e.into()),
        };

        packet_count += 1;

        // Write to savefile if saving
        if let Some(ref mut sf) = savefile {
            let packet_ref = pcap::Packet {
                header: &pkt.header,
                data: &pkt.data,
            };
            sf.write(&packet_ref);
        }

        // Display output
        if !opts.quiet {
            let timestamp = format_timestamp(&pkt.header.ts);
            let (src, dst) = extract_ip_addrs(&pkt.data, datalink)
                .unwrap_or_else(|| ("?".to_string(), "?".to_string()));

            let payload = extract_bacnet_payload(&pkt.data, datalink);
            let decoded = payload.and_then(|p| {
                if p.is_empty() {
                    None
                } else {
                    decode::decode_packet(p).ok()
                }
            });

            let (bvlc, service) = match &decoded {
                Some(d) => {
                    let s = decode::summarize(d);
                    (s.bvlc, s.service)
                }
                None => {
                    let n = payload.map_or(pkt.data.len(), |p| p.len());
                    (format!("raw({n} bytes)"), format!("raw({n} bytes)"))
                }
            };

            let detail_lines = if opts.decode {
                decoded.as_ref().map(decode::format_detail)
            } else {
                None
            };

            match opts.format {
                OutputFormat::Table => {
                    println!(
                        "{:<15} {:<24} -> {:<24} {:<24} {}",
                        timestamp, src, dst, bvlc, service
                    );
                    if let Some(ref lines) = detail_lines {
                        for line in lines {
                            println!("{line}");
                        }
                    }
                }
                OutputFormat::Json => {
                    let pj = PacketJson {
                        timestamp,
                        src,
                        dst,
                        bvlc,
                        service,
                        detail: detail_lines,
                    };
                    let json = serde_json::to_string(&pj).expect("serialize packet");
                    println!("{json}");
                }
            }
        }
    }

    // Summary on exit
    if opts.quiet {
        if let Some(ref path) = opts.save {
            eprintln!("Captured {packet_count} packets to {}", path.display());
        }
    } else {
        eprintln!("\nCaptured {packet_count} packets");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_filter_default() {
        assert_eq!(build_filter(&None), "udp port 47808");
    }

    #[test]
    fn build_filter_with_user_expr() {
        let f = build_filter(&Some("host 10.0.0.1".to_string()));
        assert_eq!(f, "udp port 47808 and (host 10.0.0.1)");
    }

    #[test]
    fn format_timestamp_midnight() {
        let ts = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };
        assert_eq!(format_timestamp(&ts), "00:00:00.000");
    }

    #[test]
    fn format_timestamp_midday() {
        let ts = libc::timeval {
            tv_sec: 45296,
            tv_usec: 789_000,
        };
        assert_eq!(format_timestamp(&ts), "12:34:56.789");
    }

    #[test]
    fn quiet_without_save_is_error() {
        let opts = CaptureOpts {
            read: None,
            save: None,
            quiet: true,
            decode: false,
            device: None,
            interface_ip: Ipv4Addr::UNSPECIFIED,
            filter: None,
            count: None,
            snaplen: 65535,
            format: OutputFormat::Table,
        };
        let err = run_capture(opts).unwrap_err();
        assert!(err.to_string().contains("--quiet requires --save"));
    }

    #[test]
    fn read_and_device_mutually_exclusive() {
        let opts = CaptureOpts {
            read: Some(PathBuf::from("test.pcap")),
            save: None,
            quiet: false,
            decode: false,
            device: Some("en0".to_string()),
            interface_ip: Ipv4Addr::UNSPECIFIED,
            filter: None,
            count: None,
            snaplen: 65535,
            format: OutputFormat::Table,
        };
        let err = run_capture(opts).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn extract_ip_addrs_ethernet() {
        // Build a minimal Ethernet + IP + UDP frame
        let mut frame = vec![0u8; 14 + 20 + 8]; // Ethernet(14) + IP(20) + UDP(8)
                                                // Ethernet header: 12 bytes dst+src, 2 bytes ethertype (0x0800 = IPv4)
        frame[12] = 0x08;
        frame[13] = 0x00;
        // IP header at offset 14
        frame[14] = 0x45; // version=4, IHL=5 (20 bytes)
                          // Source IP: 10.0.0.1 at bytes 12-15 of IP header
        frame[14 + 12] = 10;
        frame[14 + 13] = 0;
        frame[14 + 14] = 0;
        frame[14 + 15] = 1;
        // Dest IP: 10.0.0.2 at bytes 16-19 of IP header
        frame[14 + 16] = 10;
        frame[14 + 17] = 0;
        frame[14 + 18] = 0;
        frame[14 + 19] = 2;
        // UDP header at offset 14+20=34: src port 47808, dst port 47808
        frame[34] = 0xBA;
        frame[35] = 0xC0;
        frame[36] = 0xBA;
        frame[37] = 0xC0;

        let result = extract_ip_addrs(&frame, pcap::Linktype::ETHERNET);
        assert!(result.is_some());
        let (src, dst) = result.unwrap();
        assert_eq!(src, "10.0.0.1:47808");
        assert_eq!(dst, "10.0.0.2:47808");
    }

    #[test]
    fn extract_ip_addrs_too_short() {
        let frame = vec![0u8; 10];
        assert!(extract_ip_addrs(&frame, pcap::Linktype::ETHERNET).is_none());
    }

    #[test]
    fn extract_bacnet_payload_ethernet() {
        // Build Ethernet + IP + UDP frame with 4 bytes of payload
        let mut frame = vec![0u8; 14 + 20 + 8 + 4];
        frame[12] = 0x08;
        frame[13] = 0x00;
        frame[14] = 0x45; // IHL=5
                          // UDP length = 8 (header) + 4 (payload) = 12
        frame[34 + 4] = 0x00;
        frame[34 + 5] = 0x0C;
        // Payload bytes
        frame[42] = 0x81;
        frame[43] = 0x0A;
        frame[44] = 0x00;
        frame[45] = 0x04;

        let payload = extract_bacnet_payload(&frame, pcap::Linktype::ETHERNET);
        assert!(payload.is_some());
        let p = payload.unwrap();
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], 0x81);
    }

    #[test]
    fn extract_bacnet_payload_raw() {
        // DLT_RAW: no link-layer header, starts at IP
        let mut frame = vec![0u8; 20 + 8 + 2];
        frame[0] = 0x45; // IHL=5
                         // UDP length = 10
        frame[20 + 4] = 0x00;
        frame[20 + 5] = 0x0A;
        frame[28] = 0xAA;
        frame[29] = 0xBB;

        let payload = extract_bacnet_payload(&frame, pcap::Linktype(12));
        assert!(payload.is_some());
        assert_eq!(payload.unwrap(), &[0xAA, 0xBB]);
    }
}
