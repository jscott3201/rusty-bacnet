//! What a live capture lost (#2787): packets the OS dropped because its
//! buffer was full when they came, as libpcap counts them, turned into a
//! note on stderr and, in JSON, the stream's `missed` line.

use std::time::{Duration, Instant};

use super::{CaptureOpts, CaptureSource};
use crate::output;

/// How often a capture that prints no stream (`--quiet`) asks for the count:
/// it feeds only stderr's note there, and asking is a system call.
const QUIET_EVERY: Duration = Duration::from_millis(250);

/// More drops between two looks than this is no wrap of the 32-bit count
/// but a counter that went back (reset), which starts the count afresh.
const RESET_ABOVE: u32 = i32::MAX as u32;

/// The OS's drop count as the capture last saw it, and when to look again.
pub(super) struct Drops {
    /// The count when last asked.
    seen: u32,
    /// Ask at most this often; `None` for every packet.
    every: Option<Duration>,
    /// When it last asked, with an interval.
    asked: Option<Instant>,
}

impl Drops {
    /// Start from what `source` has dropped so far, once its filter is set,
    /// so drops before the capture began aren't reported. A printed stream
    /// asks before each packet: that keeps its `missed` line at or before
    /// the gap, never after (a packet read after a drop finds it counted).
    /// `--quiet` asks at most every [`QUIET_EVERY`].
    pub(super) fn start(source: &mut dyn CaptureSource, quiet: bool) -> Self {
        Self {
            seen: source.dropped().unwrap_or(0),
            every: quiet.then_some(QUIET_EVERY),
            asked: None,
        }
    }

    /// What the OS dropped since the last look, when it is time to look: a
    /// note on stderr and, in JSON, the `missed` line for the stream (none
    /// with `--quiet`, which prints no stream); nothing when it dropped
    /// none, or for a file. The OS drops what arrives while its buffer is
    /// full, after the packets it holds, so the line marks a count, not a
    /// boundary: up to a buffer of packets after it can precede the gap.
    pub(super) fn line(
        &mut self,
        source: &mut dyn CaptureSource,
        opts: &CaptureOpts,
    ) -> Option<String> {
        if !self.due(Instant::now()) {
            return None;
        }
        let count = self.since(source.dropped()?)?;
        let note = format!("Missed {count} packets (the capture buffer overflowed)");
        output::missed(&note, count.into(), opts.format).filter(|_| !opts.quiet)
    }

    /// Whether to ask at `now`: always without an interval, else once it
    /// has passed since the last time.
    fn due(&mut self, now: Instant) -> bool {
        let Some(every) = self.every else {
            return true;
        };
        if self
            .asked
            .is_some_and(|asked| now.saturating_duration_since(asked) < every)
        {
            return false;
        }
        self.asked = Some(now);
        true
    }

    /// The drops since the last look, the count now being `dropped`; `None`
    /// for none, or for a count that went back, which is taken as the new
    /// start without a note.
    fn since(&mut self, dropped: u32) -> Option<u32> {
        let count = dropped.wrapping_sub(std::mem::replace(&mut self.seen, dropped));
        (count != 0 && count <= RESET_ABOVE).then_some(count)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicBool;

    use super::super::{CaptureOpts, CaptureSource, CapturedPacket, PcapError, packets};
    use super::{Drops, QUIET_EVERY};
    use crate::output::OutputFormat;

    /// A capture source that plays `steps`, each a packet (by its second)
    /// or a timeout with the OS's drop count once it has come, then ends as
    /// a file does; the count starts at the second field.
    struct Scripted(
        std::collections::VecDeque<(Option<i64>, Option<u32>)>,
        Option<u32>,
    );

    impl CaptureSource for Scripted {
        fn next_packet(&mut self) -> Result<CapturedPacket, PcapError> {
            let (second, dropped) = self.0.pop_front().ok_or(PcapError::Eof)?;
            self.1 = dropped;
            let second = second.ok_or(PcapError::Timeout)?;
            let ts = libc::timeval {
                tv_sec: second as _,
                tv_usec: 0,
            };
            let header = pcap::PacketHeader {
                ts,
                caplen: 0,
                len: 0,
            };
            Ok(CapturedPacket {
                header,
                data: Vec::new(),
            })
        }

        fn set_filter(&mut self, _: &str) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn datalink(&self) -> pcap::Linktype {
            pcap::Linktype::ETHERNET
        }

        fn savefile(
            &self,
            _: &std::path::Path,
        ) -> Result<pcap::Savefile, Box<dyn std::error::Error>> {
            Err("no savefile".into())
        }

        fn dropped(&mut self) -> Option<u32> {
            self.1
        }
    }

    /// What `packets` prints in `format` (with `--quiet` when `quiet`) for
    /// `steps` from a drop count of `start`: each line's packet second, or
    /// `missed:N`.
    fn printed(
        start: Option<u32>,
        steps: &[(Option<i64>, Option<u32>)],
        format: OutputFormat,
        quiet: bool,
    ) -> (Vec<String>, u64) {
        let opts = CaptureOpts {
            read: None,
            save: None,
            quiet,
            decode: false,
            device: None,
            interface_ip: Ipv4Addr::UNSPECIFIED,
            port: 47808,
            filter: None,
            count: None,
            snaplen: 65535,
            format,
        };
        let mut source = Scripted(steps.iter().copied().collect(), start);
        let mut stdout = Vec::new();
        let running = AtomicBool::new(true);
        let datalink = pcap::Linktype::ETHERNET;
        let read = packets(
            &mut source,
            &mut None,
            datalink,
            &opts,
            &running,
            &mut stdout,
        );
        let text = String::from_utf8(stdout).unwrap();
        let lines = text.lines().map(|line| match format {
            OutputFormat::Json => {
                let json: serde_json::Value = serde_json::from_str(line).unwrap();
                match json["type"].as_str() {
                    Some("packet") => json["timestamp"].as_str().unwrap().to_string(),
                    Some("missed") => {
                        assert_eq!(json.as_object().unwrap().len(), 2, "{line}");
                        format!("missed:{}", json["count"])
                    }
                    _ => panic!("{line}"),
                }
            }
            OutputFormat::Table => line.split(' ').next().unwrap().to_string(),
        });
        (lines.collect(), read.unwrap())
    }

    /// Packets the OS dropped are a `missed` line in JSON (#2787), before
    /// the packet the capture reads next or at a quiet second's timeout,
    /// counting from the count when the capture began, what was dropped
    /// since the last look, across the count's wrap too; a count that went
    /// back starts afresh without a line. Every line is one JSON value. A
    /// table's stdout and `--quiet`'s are unchanged (the note is on
    /// stderr), as is a file's stream, which can't drop.
    #[test]
    fn dropped_packets_are_marked_in_the_stream() {
        let start = Some(u32::MAX - 2);
        let steps = [
            (Some(1), Some(u32::MAX - 2)),
            (Some(2), Some(u32::MAX)),
            (None, Some(1)),
            (Some(3), Some(1)),
            // Went back: a reset, not 4 billion drops.
            (Some(4), Some(0)),
            (Some(5), Some(3)),
            (None, Some(4)),
        ];
        let at = |second: u32| format!("00:00:0{second}.000");
        let missed = |count: u32| format!("missed:{count}");
        let (json, read) = printed(start, &steps, OutputFormat::Json, false);
        let expected = [
            at(1),
            missed(2),
            at(2),
            missed(2),
            at(3),
            at(4),
            missed(3),
            at(5),
            missed(1),
        ];
        assert_eq!(json, expected);
        assert_eq!(read, 5);

        let packets_only = [at(1), at(2), at(3), at(4), at(5)];
        let (table, _) = printed(start, &steps, OutputFormat::Table, false);
        assert_eq!(table, packets_only);
        let (quiet, read) = printed(start, &steps, OutputFormat::Json, true);
        assert!(quiet.is_empty(), "{quiet:?}");
        assert_eq!(read, 5);
        let file: Vec<_> = steps.iter().map(|&(second, _)| (second, None)).collect();
        assert_eq!(
            printed(None, &file, OutputFormat::Json, false).0,
            packets_only
        );
    }

    /// A printed stream asks for the count before every packet; `--quiet`,
    /// whose count feeds only stderr, at most every 250 ms.
    #[test]
    fn quiet_asks_at_most_every_250_ms() {
        use std::time::{Duration, Instant};

        let mut source = Scripted(Default::default(), Some(7));
        let at = Instant::now();
        let mut printing = Drops::start(&mut source, false);
        assert_eq!(printing.seen, 7);
        assert!((0..3).all(|_| printing.due(at)));

        let mut quiet = Drops::start(&mut source, true);
        assert_eq!(QUIET_EVERY, Duration::from_millis(250));
        assert!(quiet.due(at));
        assert!(!quiet.due(at));
        assert!(!quiet.due(at + QUIET_EVERY - Duration::from_millis(1)));
        assert!(quiet.due(at + QUIET_EVERY));
        assert!(!quiet.due(at + QUIET_EVERY + Duration::from_millis(1)));
        assert!(quiet.due(at + QUIET_EVERY * 3));
    }
}
