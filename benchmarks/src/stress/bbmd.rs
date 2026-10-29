//! BBMD forwarding capacity stress scenario.
//!
//! Registers N foreign devices with a BBMD, then has every one send
//! broadcasts through it (Distribute-Broadcast-To-Network) as fast as its
//! socket takes them, with no pacing, for the step's duration. The BBMD's
//! fan-out limits are raised far above what loopback can carry, so the rate
//! it forwards at is its capacity, and its queue overflows show where that
//! runs out.
//!
//! On Linux each foreign device binds its own loopback address (127.1.x.y),
//! so the BBMD sees one source address per device, as on a real network,
//! and its per-address registration guard doesn't cap the device count.
//! Other hosts have only 127.0.0.1 without configuration; there the devices
//! share it, and only as many register as that guard admits in a second.
//! Run the scenario in a Linux container (`examples/docker`) for the figure.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BipTransport, FanoutCounters, FanoutPolicy, ForeignDeviceConfig};
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::port::TransportPort;

use crate::stress::harness::current_rss_kb;
use crate::stress::output::{DegradationPoint, LatencyRecorder, rate};

/// Whether each foreign device gets its own loopback address: Linux routes
/// all of 127.0.0.0/8 to the loopback interface.
pub const DISTINCT_ADDRESSES: bool = cfg!(target_os = "linux");

/// The UDP port every foreign device binds on its own address.
const FOREIGN_PORT: u16 = 47_850;

/// Foreign device `i`'s loopback address: 127.1.0.1, 127.1.0.2, and so on,
/// 250 to a third octet.
pub fn foreign_address(i: u64) -> Ipv4Addr {
    let third = (i / 250) as u8;
    let fourth = (i % 250) as u8 + 1;
    Ipv4Addr::new(127, 1, third, fourth)
}

fn foreign_transport(i: u64) -> BipTransport {
    if DISTINCT_ADDRESSES {
        let ip = foreign_address(i);
        // Bind the device's own address, so its datagrams leave from it.
        let mut fd = BipTransport::new(ip, FOREIGN_PORT, ip);
        fd.set_share_port_by_address(true);
        fd
    } else {
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
    }
}

/// The forwarded-packet counters' growth from `before` to `after`.
fn growth(before: &FanoutCounters, after: &FanoutCounters) -> FanoutCounters {
    FanoutCounters {
        packets_forwarded: after.packets_forwarded - before.packets_forwarded,
        bytes_forwarded: after.bytes_forwarded - before.bytes_forwarded,
        packets_throttled: after.packets_throttled - before.packets_throttled,
        destinations_deduplicated: after.destinations_deduplicated
            - before.destinations_deduplicated,
        queue_overflow_drops: after.queue_overflow_drops - before.queue_overflow_drops,
        send_errors: after.send_errors - before.send_errors,
    }
}

pub async fn run(duration_secs: u64, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    if !DISTINCT_ADDRESSES {
        eprintln!(
            "note: the foreign devices share 127.0.0.1 on this host, so the BBMD's \
             per-address guard limits how many register; run it on Linux for one \
             address each"
        );
    }

    for &count in steps {
        eprintln!("--- {} foreign devices ---", count);

        let rss_before = current_rss_kb();

        // A BBMD whose limits sit far above what loopback carries.
        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd.enable_bbmd(vec![]);
        bbmd.enable_foreign_device_registration(ForeignDevicePolicy {
            max_entries_per_source: 256,
            max_fdt_fanout: 256,
            registration_rate_per_source: 1000,
            registration_rate_global: 2000,
            ..Default::default()
        });
        bbmd.set_fanout_policy(FanoutPolicy {
            max_fanout_per_input: 256,
            max_packets_per_sec_global: 100_000_000,
            max_bytes_per_sec_global: 10_000_000_000,
            max_packets_per_sec_per_origin: 100_000_000,
            queue_capacity: 4096,
        });
        let mut bbmd_rx = bbmd.start().await.unwrap();
        let bbmd_mac = bbmd.local_mac().to_vec();
        let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
        // The BBMD's own receive channel isn't measured; keep it drained.
        let bbmd_drain = tokio::spawn(async move { while bbmd_rx.recv().await.is_some() {} });

        // Register N foreign devices, each counting what reaches it.
        let received = Arc::new(AtomicU64::new(0));
        let mut fds = Vec::new();
        let mut drains = Vec::new();
        let mut fd_errors = 0u64;
        for i in 0..count {
            let mut fd = foreign_transport(i);
            fd.register_as_foreign_device(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::from(bbmd_ip),
                bbmd_port,
                renewal_interval: None,
                ttl: 600,
            });
            match fd.start().await {
                Ok(mut rx) => {
                    let counter = received.clone();
                    drains.push(tokio::spawn(async move {
                        while rx.recv().await.is_some() {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }));
                    fds.push(fd);
                }
                Err(e) => {
                    eprintln!("  foreign device {i} failed to start: {e}");
                    fd_errors += 1;
                }
            }
        }

        // Wait for the registrations, up to 2 seconds.
        let fdt_len = || {
            bbmd.bbmd_state().map_or(0, |state| {
                state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fdt()
                    .len()
            })
        };
        let wait_until = Instant::now() + Duration::from_secs(2);
        while fdt_len() < fds.len() && Instant::now() < wait_until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let registered = fdt_len();
        eprintln!("  FDT entries: {registered} (of {count})");

        // Every device floods broadcasts through the BBMD until the deadline.
        let test_npdu: Arc<[u8]> = Arc::from(&[0x01, 0x00, 0x10, 0x08][..]); // NPDU + Who-Is
        let received_before = received.load(Ordering::Relaxed);
        let before = bbmd.fanout_counters();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(duration_secs);
        let mut senders = Vec::new();
        for fd in fds {
            let npdu = test_npdu.clone();
            senders.push(tokio::spawn(async move {
                let mut recorder = LatencyRecorder::new();
                while Instant::now() < deadline {
                    let start = Instant::now();
                    match fd.send_broadcast(&npdu).await {
                        Ok(()) => recorder.record_success(start),
                        Err(_) => recorder.record_failure(),
                    }
                    // Let the BBMD and the receivers run between sends on a
                    // busy runtime, without a fixed pause.
                    tokio::task::yield_now().await;
                }
                (fd, recorder)
            }));
        }
        let mut sends = LatencyRecorder::new();
        let mut fds = Vec::new();
        for s in senders {
            if let Ok((fd, recorder)) = s.await {
                sends.merge(&recorder);
                fds.push(fd);
            }
        }
        // The step's time ends with the sends; the queue then gets a moment
        // to empty, off the clock, before the counters are read.
        let elapsed = started.elapsed();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let fanout = growth(&before, &bbmd.fanout_counters());
        let delivered = received.load(Ordering::Relaxed) - received_before;
        let rss_after = current_rss_kb();

        let send_stats = sends.stats();
        eprintln!(
            "  broadcasts sent={} failed={} ({:.0} per second), send call p50={} µs p99={} µs",
            sends.successful(),
            sends.failed(),
            rate(sends.successful(), elapsed),
            send_stats.p50,
            send_stats.p99,
        );
        eprintln!(
            "  BBMD forwarded={} ({:.0} per second, {} bytes) throttled={} \
             queue_overflow_drops={} send_errors={}",
            fanout.packets_forwarded,
            rate(fanout.packets_forwarded, elapsed),
            fanout.bytes_forwarded,
            fanout.packets_throttled,
            fanout.queue_overflow_drops,
            fanout.send_errors,
        );
        eprintln!(
            "  delivered to foreign devices={} ({:.0} per second) | memory: {} KB to {} KB",
            delivered,
            rate(delivered, elapsed),
            rss_before,
            rss_after,
        );

        // The step's rate is the BBMD's: forwarded packets per second.
        curve.push(DegradationPoint {
            parameter: count,
            p50_us: send_stats.p50,
            p99_us: send_stats.p99,
            throughput: rate(fanout.packets_forwarded, elapsed),
            successful: fanout.packets_forwarded,
            errors: sends.failed() + fd_errors + fanout.queue_overflow_drops + fanout.send_errors,
            elapsed_secs: elapsed.as_secs_f64(),
        });

        // Cleanup
        for mut fd in fds {
            let _ = fd.stop().await;
        }
        for d in drains {
            d.abort();
        }
        let _ = bbmd.stop().await;
        bbmd_drain.abort();
    }

    curve
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_addresses_are_distinct_loopback_hosts() {
        assert_eq!(foreign_address(0), Ipv4Addr::new(127, 1, 0, 1));
        assert_eq!(foreign_address(249), Ipv4Addr::new(127, 1, 0, 250));
        assert_eq!(foreign_address(250), Ipv4Addr::new(127, 1, 1, 1));
        let all: std::collections::HashSet<_> = (0..1000).map(foreign_address).collect();
        assert_eq!(all.len(), 1000);
        assert!(all.iter().all(|ip| ip.is_loopback()));
    }
}
