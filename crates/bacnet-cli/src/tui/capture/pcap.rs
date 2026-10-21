//! pcap export: the classic libpcap file format, one file per link type.
//!
//! - B/IP as Ethernet (LINKTYPE 1) or raw IPv4 (LINKTYPE 228). The tap
//!   hands over the BVLL message only, the UDP payload, so the IPv4 and UDP
//!   headers (and for Ethernet a frame header) are made up from the
//!   endpoints the frame was captured with. The UDP checksum is zero, which
//!   IPv4 allows to mean none; the IPv4 header checksum is computed.
//! - MS/TP as LINKTYPE 165, each frame from its preamble.
//!
//! B/IPv6, BACnet/SC and the monitor's line errors are left out: B/IPv6
//! would need IPv6 headers, and SC needs pcapng with its own encapsulation
//! (a follow-up).
//!
//! Files are streamed through a buffered writer, never replace a file, and
//! are removed again if writing them fails.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CaptureFrame, Direction, ExportJob, Framing};

/// LINKTYPE_ETHERNET.
pub(crate) const LINKTYPE_ETHERNET: u32 = 1;
/// LINKTYPE_BACNET_MS_TP.
pub(crate) const LINKTYPE_MSTP: u32 = 165;
/// LINKTYPE_IPV4.
pub(crate) const LINKTYPE_IPV4: u32 = 228;

const SNAPLEN: u32 = 65_535;

/// Names tried after `STEM-LINK.pcap` is taken: `STEM-LINK-1.pcap` and on.
const RETRIES: u32 = 100;

/// The file header: microsecond timestamps, version 2.4, little-endian.
fn file_header(out: &mut impl Write, link_type: u32) -> io::Result<()> {
    out.write_all(&0xA1B2_C3D4_u32.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&4u16.to_le_bytes())?;
    out.write_all(&0i32.to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&SNAPLEN.to_le_bytes())?;
    out.write_all(&link_type.to_le_bytes())
}

fn record(out: &mut impl Write, time: SystemTime, parts: &[&[u8]]) -> io::Result<()> {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = u32::try_from(since.as_secs()).unwrap_or(u32::MAX);
    let length: usize = parts.iter().map(|part| part.len()).sum();
    let length = u32::try_from(length).unwrap_or(u32::MAX);
    out.write_all(&seconds.to_le_bytes())?;
    out.write_all(&since.subsec_micros().to_le_bytes())?;
    out.write_all(&length.to_le_bytes())?;
    out.write_all(&length.to_le_bytes())?;
    for part in parts {
        out.write_all(part)?;
    }
    Ok(())
}

/// The one's-complement sum of a header, as IPv4 checksums it.
fn checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = header
        .chunks(2)
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])))
        .sum();
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn v4(address: SocketAddr) -> Option<(Ipv4Addr, u16)> {
    match address.ip() {
        IpAddr::V4(ip) => Some((ip, address.port())),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(|ip| (ip, address.port())),
    }
}

/// The IPv4 and UDP headers, 28 octets, for a datagram holding `payload`
/// octets; `None` without IPv4 endpoints or for a payload too long.
pub(crate) fn ipv4_udp_header(
    source: SocketAddr,
    destination: SocketAddr,
    id: u16,
    payload: usize,
) -> Option<[u8; 28]> {
    let ((src_ip, src_port), (dst_ip, dst_port)) = (v4(source)?, v4(destination)?);
    let udp_length = u16::try_from(8 + payload).ok()?;
    let total = u16::try_from(20 + 8 + payload).ok()?;
    let mut header = [0u8; 28];
    header[..2].copy_from_slice(&[0x45, 0]);
    header[2..4].copy_from_slice(&total.to_be_bytes());
    header[4..6].copy_from_slice(&id.to_be_bytes());
    // Don't fragment; time to live 64; UDP.
    header[6..10].copy_from_slice(&[0x40, 0, 64, 17]);
    header[12..16].copy_from_slice(&src_ip.octets());
    header[16..20].copy_from_slice(&dst_ip.octets());
    let sum = checksum(&header[..20]);
    header[10..12].copy_from_slice(&sum.to_be_bytes());
    header[20..22].copy_from_slice(&src_port.to_be_bytes());
    header[22..24].copy_from_slice(&dst_port.to_be_bytes());
    header[24..26].copy_from_slice(&udp_length.to_be_bytes());
    Some(header)
}

/// A made-up Ethernet MAC for the destination: the IPv4 multicast mapping
/// (`01:00:5e` and the address's low 23 bits), broadcast for the limited
/// broadcast address or for an Original-Broadcast-NPDU this node sent or
/// the wire carried, and otherwise a locally administered unicast MAC that
/// carries the address.
fn destination_mac(ip: Ipv4Addr, frame: &CaptureFrame) -> [u8; 6] {
    let [a, b, c, d] = ip.octets();
    if ip.is_multicast() {
        return [0x01, 0x00, 0x5E, b & 0x7F, c, d];
    }
    let sent_broadcast =
        frame.summary.link == "original-broadcast-npdu" && frame.direction != Direction::In;
    if ip.is_broadcast() || sent_broadcast {
        return [0xFF; 6];
    }
    [0x02, 0x00, a, b, c, d]
}

fn source_mac(ip: Ipv4Addr) -> [u8; 6] {
    let [a, b, c, d] = ip.octets();
    [0x02, 0x00, a, b, c, d]
}

/// Which file of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ethernet,
    Ipv4,
    Mstp,
}

impl Kind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Ethernet => "bip",
            Self::Ipv4 => "ipv4",
            Self::Mstp => "mstp",
        }
    }

    fn takes(self, frame: &CaptureFrame) -> bool {
        match self {
            Self::Ethernet | Self::Ipv4 => frame.framing == Framing::Bip && frame.udp.is_some(),
            Self::Mstp => frame.framing == Framing::Mstp && !frame.bytes.is_empty(),
        }
    }
}

/// Write the frames `kind` takes as one pcap file; returns how many.
fn write(out: &mut impl Write, frames: &[Arc<CaptureFrame>], kind: Kind) -> io::Result<usize> {
    let link_type = match kind {
        Kind::Ethernet => LINKTYPE_ETHERNET,
        Kind::Ipv4 => LINKTYPE_IPV4,
        Kind::Mstp => LINKTYPE_MSTP,
    };
    file_header(out, link_type)?;
    let mut written = 0usize;
    for frame in frames.iter().filter(|frame| kind.takes(frame)) {
        if kind == Kind::Mstp {
            record(out, frame.time, &[&frame.bytes])?;
            written += 1;
            continue;
        }
        let Some((source, destination)) = frame.udp else {
            continue;
        };
        let id = u16::try_from(written & 0xFFFF).unwrap_or_default();
        let Some(ip) = ipv4_udp_header(source, destination, id, frame.bytes.len()) else {
            continue;
        };
        if kind == Kind::Ipv4 {
            record(out, frame.time, &[&ip, &frame.bytes])?;
        } else {
            let (Some((src, _)), Some((dst, _))) = (v4(source), v4(destination)) else {
                continue;
            };
            let mut ethernet = [0u8; 14];
            ethernet[..6].copy_from_slice(&destination_mac(dst, frame));
            ethernet[6..12].copy_from_slice(&source_mac(src));
            ethernet[12..].copy_from_slice(&[0x08, 0x00]);
            record(out, frame.time, &[&ethernet, &ip, &frame.bytes])?;
        }
        written += 1;
    }
    Ok(written)
}

/// The frames of `frames` that `raw_ipv4` picks for its B/IP file, as one
/// pcap file in memory; for tests.
#[cfg(test)]
pub(crate) fn bip_file(frames: &[Arc<CaptureFrame>], raw_ipv4: bool) -> Option<Vec<u8>> {
    let kind = if raw_ipv4 { Kind::Ipv4 } else { Kind::Ethernet };
    let mut out = Vec::new();
    (write(&mut out, frames, kind).ok()? > 0).then_some(out)
}

/// The same for MS/TP.
#[cfg(test)]
pub(crate) fn mstp_file(frames: &[Arc<CaptureFrame>]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    (write(&mut out, frames, Kind::Mstp).ok()? > 0).then_some(out)
}

/// What an export wrote, and the error that stopped it, if one did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Exported {
    /// The files written in full.
    pub(crate) written: Vec<String>,
    /// Why the rest wasn't.
    pub(crate) error: Option<String>,
}

/// Write `job` into `dir`: `STEM-bip.pcap` (Ethernet) or `STEM-ipv4.pcap`
/// (raw IPv4), and `STEM-mstp.pcap`, for the links it has frames of. A name
/// that is taken gets `-1`, `-2` and on. Blocking.
pub(crate) fn export(job: &ExportJob, dir: &Path) -> Exported {
    let bip = if job.raw_ipv4 {
        Kind::Ipv4
    } else {
        Kind::Ethernet
    };
    let mut exported = Exported::default();
    for kind in [bip, Kind::Mstp] {
        if !job.frames.iter().any(|frame| kind.takes(frame)) {
            continue;
        }
        match write_file(dir, &job.stem, kind, &job.frames) {
            Ok(path) => exported.written.push(path.display().to_string()),
            Err(error) => {
                exported.error = Some(error);
                return exported;
            }
        }
    }
    if exported.written.is_empty() {
        exported.error = Some("nothing to export: no B/IP or MS/TP frames in the list".into());
    }
    exported
}

/// Create a new file for `kind`, write it, and remove it if that fails.
fn write_file(
    dir: &Path,
    stem: &str,
    kind: Kind,
    frames: &[Arc<CaptureFrame>],
) -> Result<PathBuf, String> {
    let (path, file) = create(dir, stem, kind.suffix())?;
    let mut out = BufWriter::new(file);
    let written = write(&mut out, frames, kind)
        .and_then(|_| out.flush())
        .and_then(|()| out.get_ref().sync_all());
    match written {
        Ok(()) => Ok(path),
        Err(error) => {
            drop(out);
            let _ = std::fs::remove_file(&path);
            Err(format!("{}: {error}", path.display()))
        }
    }
}

/// `STEM-SUFFIX.pcap`, or the first of `STEM-SUFFIX-N.pcap` not taken.
fn create(dir: &Path, stem: &str, suffix: &str) -> Result<(PathBuf, File), String> {
    for attempt in 0..=RETRIES {
        let name = match attempt {
            0 => format!("{stem}-{suffix}.pcap"),
            n => format!("{stem}-{suffix}-{n}.pcap"),
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "{}: {RETRIES} files of that name exist",
        dir.join(format!("{stem}-{suffix}.pcap")).display()
    ))
}
