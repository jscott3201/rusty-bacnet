//! Explicit opt-in, receiver-isolated RSS experiment for SC WebSocket limits.
//! Build this identical harness in both release worktrees, then provide the two
//! test executables via SC_WS_BASELINE_BIN / SC_WS_CANDIDATE_BIN. Invoke this test
//! with --ignored --exact sc_ws_memory --nocapture and SC_WS_MODE=calibration or
//! acceptance. SC_WS_OUTPUT names a fresh directory. Never run alongside builds.
//! Child certificate material travels only over private stdin; output has no keys.

use bacnet_benchmarks::sc_helpers::*;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "sc_ws_memory/peer.rs"]
mod peer;
#[path = "sc_ws_memory/sampler.rs"]
mod sampler;
#[path = "sc_ws_memory/victim.rs"]
mod victim;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
#[ignore = "explicit isolated-process release RSS experiment; not an ordinary test gate"]
fn sc_ws_memory() {
    assert!(
        !cfg!(debug_assertions),
        "memory experiments require --release"
    );
    if let Ok(role) = std::env::var("SC_WS_VICTIM") {
        victim::run(&role);
        return;
    }
    let mode = std::env::var("SC_WS_MODE").expect("set calibration or acceptance");
    assert!(matches!(mode.as_str(), "calibration" | "acceptance"));
    let output = PathBuf::from(std::env::var("SC_WS_OUTPUT").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    let variants = [
        (
            "baseline",
            PathBuf::from(std::env::var("SC_WS_BASELINE_BIN").unwrap()),
        ),
        (
            "candidate",
            PathBuf::from(std::env::var("SC_WS_CANDIDATE_BIN").unwrap()),
        ),
    ];
    let (connections, trials) = if mode == "calibration" {
        (1, 1)
    } else {
        (16, 5)
    };
    let mut results = Vec::new();
    let runtime = runtime();
    let mut sequence = 0;
    for role in ["hub", "node"] {
        for workload in ["header", "fragments"] {
            for trial in 0..trials {
                let order = if trial % 2 == 0 { [0, 1] } else { [1, 0] };
                for variant in order {
                    let (label, binary) = &variants[variant];
                    let name = format!("{label}-{role}-{workload}-n{connections}-t{trial}");
                    let path = output.join(format!("{name}.json"));
                    assert!(!path.exists(), "refusing to overwrite trial evidence");
                    let start = Instant::now();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        runtime
                            .block_on(async {
                                tokio::time::timeout(
                                    Duration::from_secs(25),
                                    trial_run(binary, role, workload, connections, &output, &name),
                                )
                                .await
                            })
                            .unwrap()
                    }));
                    let mut result = match result {
                        Ok(value) => value,
                        Err(_) => json!({"status":"incomplete"}),
                    };
                    result["revision"] = json!(label);
                    result["role"] = json!(role);
                    result["workload"] = json!(workload);
                    result["connections"] = json!(connections);
                    result["trial"] = json!(trial);
                    result["sequence"] = json!(sequence);
                    result["duration_ms"] = json!(start.elapsed().as_millis());
                    result["victim_binary"] = json!(binary);
                    if result["status"] == "complete" {
                        let expected = if *label == "candidate" {
                            connections
                        } else {
                            0
                        };
                        result["rejection_pass"] = json!(result["rejected_peers"] == expected);
                    }
                    if *label == "candidate" && result["status"] == "complete" {
                        let envelope = if connections == 1 { 17 } else { 32 } * 1024 * 1024;
                        result["envelope_bytes"] = json!(envelope);
                        result["envelope_pass"] =
                            json!(result["attack_delta_bytes"].as_u64().unwrap() <= envelope);
                    }
                    std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
                    println!(
                        "{name}: status={} delta={} written={} rejected={}",
                        result["status"],
                        result["attack_delta_bytes"],
                        result["body_bytes_written"],
                        result["rejected_peers"]
                    );
                    results.push(result);
                    sequence += 1;
                }
            }
        }
    }
    std::fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    assert!(
        results.iter().all(|r| r["status"] == "complete"
            && r["envelope_pass"] != false
            && r["rejection_pass"] == true),
        "incomplete trial or predeclared RSS envelope failure; retained all evidence"
    );
}

struct Victim(Child);
impl Drop for Victim {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn trial_run(
    binary: &Path,
    role: &str,
    workload: &str,
    count: usize,
    output: &Path,
    name: &str,
) -> Value {
    let certs = generate_test_certs();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let stderr = std::fs::File::create(output.join(format!("{name}-victim.log"))).unwrap();
    let child = Command::new(binary)
        .args(["--exact", "sc_ws_memory", "--ignored", "--nocapture"])
        .env("SC_WS_VICTIM", role)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .unwrap();
    let mut victim = Victim(child);
    let pid = victim.0.id();
    let mut input = victim.0.stdin.take().unwrap();
    // Do not log this setup record: it includes ephemeral client/server keys.
    writeln!(input, "{}", json!({"url":url,"ca":certs.ca_cert_pem,"server_cert":certs.server_cert_pem,"server_key":certs.server_key_pem,"client_cert":certs.client_cert_pem,"client_key":certs.client_key_pem})).unwrap();
    let stdout = victim.0.stdout.take().unwrap();
    let (ready, _stdout) = tokio::task::spawn_blocking(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "victim exited without readiness"
            );
            if let Some(offset) = line.find("SC_MEMORY ") {
                return (
                    serde_json::from_str::<Value>(&line[offset + 10..]).unwrap(),
                    reader,
                );
            }
        }
    })
    .await
    .unwrap();
    let hub_url = ready["url"].as_str().unwrap_or_default();
    let mut samples = sampler::Sampler::start(pid, &output.join(format!("{name}-samples.jsonl")));
    let warm = open_peer(role, hub_url, &listener, &certs, &mut input, 1).await;
    drop(warm);
    tokio::time::sleep(Duration::from_millis(100)).await;
    samples.phase(1); // warmed idle, after benign connection cleanup
    tokio::time::sleep(Duration::from_millis(200)).await;
    samples.phase(0); // Connection setup is outside warmed-idle statistics.
    let mut peers = Vec::new();
    for id in 0..count {
        peers.push(open_peer(role, hub_url, &listener, &certs, &mut input, id as u8 + 2).await);
    }
    samples.phase(2); // established N idle, all mTLS + Connect + heartbeat complete
    tokio::time::sleep(Duration::from_millis(200)).await;
    samples.phase(3);
    let attacked =
        futures_util::future::join_all(peers.into_iter().map(|peer| peer.attack(workload))).await;
    // Fixed residence interval exposes baseline partial-frame/message storage.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let peers = futures_util::future::join_all(attacked.into_iter().map(
        |(mut peer, mut stats)| async move {
            let evidence = peer.rejection_evidence().await;
            stats["rejected"] = evidence["rejected"].clone();
            stats["rejection_evidence"] = evidence;
            (peer, stats)
        },
    ))
    .await;
    let attack_stats: Vec<_> = peers.iter().map(|(_, value)| value.clone()).collect();
    drop(peers);
    samples.phase(4);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let raw = samples.finish();
    writeln!(input, "stop").unwrap();
    let status = victim.0.wait().unwrap();
    assert!(status.success(), "victim failed: {status}");
    assert!(
        !raw.iter().any(|sample| sample.get("error").is_some()),
        "PID sampling failed or emergency cap exceeded"
    );
    let mut phase_stats = Vec::new();
    for phase in 1..=4 {
        let mut rss: Vec<_> = raw
            .iter()
            .filter(|r| r["phase"] == phase)
            .map(|r| r["rss_bytes"].as_u64().unwrap())
            .collect();
        assert!(rss.len() >= 10, "insufficient samples for phase {phase}");
        rss.sort_unstable();
        phase_stats.push(json!({"phase":phase,"samples":rss.len(),"min":rss[0],"median":rss[rss.len()/2],"max":rss[rss.len()-1]}));
    }
    let established = phase_stats[1]["median"].as_u64().unwrap();
    let peak = phase_stats[2]["max"].as_u64().unwrap();
    json!({"status":if attack_stats.iter().any(|s| s["write_incomplete"] == true || s["rejection_evidence"]["status"] == "unexpected_output") {"incomplete"} else {"complete"},"pid":pid,"sample_cadence_ms":10,"tokio_workers":2,"offered_body_limit_per_peer":8388608,"baseline_residence_ms":1000,"phases":phase_stats,"attack_delta_bytes":peak.saturating_sub(established),"body_bytes_written":attack_stats.iter().map(|v|v["body_bytes_written"].as_u64().unwrap()).sum::<u64>(),"rejected_peers":attack_stats.iter().filter(|v|v["rejected"]==true).count(),"peers":attack_stats,"samples_file":format!("{name}-samples.jsonl")})
}

async fn open_peer(
    role: &str,
    hub_url: &str,
    listener: &tokio::net::TcpListener,
    certs: &CertMaterial,
    input: &mut std::process::ChildStdin,
    id: u8,
) -> peer::Peer {
    tokio::time::timeout(Duration::from_secs(5), async {
        if role == "hub" {
            peer::Peer::hub(hub_url, certs, id).await
        } else {
            writeln!(input, "dial").unwrap();
            peer::Peer::node(listener, certs).await
        }
    })
    .await
    .unwrap()
}
