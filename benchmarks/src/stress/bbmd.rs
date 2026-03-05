//! BBMD foreign device stress scenario.
//!
//! Registers increasing numbers of foreign devices with a BBMD and
//! tests broadcast distribution performance.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::port::TransportPort;

use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(duration_secs: u64, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();

    for &count in steps {
        eprintln!("--- {} foreign devices ---", count);

        // Start BBMD
        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        bbmd.enable_bbmd(vec![]);
        let _bbmd_rx = bbmd.start().await.unwrap();
        let bbmd_mac = bbmd.local_mac().to_vec();
        let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

        // Register N foreign devices
        let mut fds = Vec::new();
        let mut fd_errors = 0u64;

        for _ in 0..count {
            let mut fd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
            fd.register_as_foreign_device(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::from(bbmd_ip),
                bbmd_port,
                ttl: 600,
            });
            match fd.start().await {
                Ok(_rx) => fds.push(fd),
                Err(e) => {
                    eprintln!("  FD registration error: {}", e);
                    fd_errors += 1;
                }
            }
        }

        // Wait for registrations to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify FDT size
        if let Some(state) = bbmd.bbmd_state() {
            let mut st = state.lock().await;
            let fdt_len = st.fdt().len();
            eprintln!("  FDT entries: {} (expected {})", fdt_len, count);
        }

        // Measure broadcast distribution: each FD sends a broadcast
        let mut recorder = LatencyRecorder::new();
        let deadline = Instant::now() + Duration::from_secs(duration_secs);
        let test_npdu = vec![0x01, 0x00, 0x10, 0x08]; // Minimal NPDU + WhoIs

        let mut sends = 0u64;
        while Instant::now() < deadline && !fds.is_empty() {
            let fd_idx = sends as usize % fds.len();
            let start = Instant::now();
            match fds[fd_idx].send_broadcast(&test_npdu).await {
                Ok(_) => {
                    recorder.record_success(start);
                    sends += 1;
                }
                Err(_) => recorder.record_failure(),
            }
            // Small delay to avoid overwhelming
            tokio::time::sleep(Duration::from_micros(100)).await;
        }

        let stats = recorder.stats();
        let throughput = recorder.successful() as f64 / duration_secs as f64;

        eprintln!(
            "  sends={} errors={} throughput={:.0}/s p50={}µs p99={}µs",
            recorder.successful(),
            recorder.failed() + fd_errors,
            throughput,
            stats.p50,
            stats.p99,
        );

        curve.push(DegradationPoint {
            parameter: count,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput,
            errors: recorder.failed() + fd_errors,
        });

        // Cleanup
        for mut fd in fds {
            let _ = fd.stop().await;
        }
        let _ = bbmd.stop().await;
    }

    curve
}
