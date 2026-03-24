# Rusty BACnet — Benchmarks & Stress Test Results

> As of version 0.8.0 | 2026-03-23
>
> **Hardware:** MacBook Pro (Mac17,2) — Apple M5, 10-core (4P + 6E), 16 GB RAM
>
> **Software:** macOS 26.4 | Rust 1.94.0 | Release mode | TLS provider: aws-lc-rs
>
> All tests ran on localhost with zero errors unless noted.

---

## 1. Criterion Micro-Benchmarks

### 1.1 Encoding / Decoding (CPU-bound, no I/O)

| Benchmark | Time |
|---|---|
| `encode_read_property_request` | **19.1 ns** |
| `decode_read_property_request` | **4.3 ns** |
| `encode_npdu_100b_payload` | **16.1 ns** |
| `decode_npdu_100b_payload` | **10.4 ns** |
| `encode_apdu_confirmed_request` | **18.9 ns** |
| `decode_apdu_confirmed_request` | **8.7 ns** |
| `full_stack_encode_rp` | **82.4 ns** |

### 1.2 BACnet/IP (BIP) — UDP Transport

#### Latency (single request round-trip)

| Operation | Latency |
|---|---|
| ReadProperty | **20.9 µs** |
| WriteProperty | **21.1 µs** |
| RPM (10 objects) | **24.7 µs** |

#### Throughput (batched requests)

| Operation | 10 ops | 100 ops | 1000 ops | Peak ops/s |
|---|---|---|---|---|
| ReadProperty | 210 µs | 2.10 ms | 20.7 ms | **~48.3 K/s** |
| WriteProperty | 207 µs | 2.07 ms | 20.7 ms | **~48.3 K/s** |

### 1.3 BACnet/IPv6 (BIP6) — UDP Transport

#### Latency

| Operation | Latency |
|---|---|
| ReadProperty | **21.0 µs** |
| WriteProperty | **20.9 µs** |
| RPM (10 objects) | **22.3 µs** |

#### Throughput

| Operation | 10 ops | 100 ops | 1000 ops | Peak ops/s |
|---|---|---|---|---|
| ReadProperty | 210 µs | 2.10 ms | 21.1 ms | **~47.4 K/s** |
| WriteProperty | 230 µs | 2.29 ms | 22.9 ms | **~43.7 K/s** |

### 1.4 BACnet/SC — TLS WebSocket (Server Auth Only)

#### Latency

| Operation | Latency |
|---|---|
| ReadProperty | **46.6 µs** |
| WriteProperty | **47.3 µs** |
| RPM (10 objects) | **51.9 µs** |

#### Throughput

| Operation | 10 ops | 100 ops | 1000 ops | Peak ops/s |
|---|---|---|---|---|
| ReadProperty | 467 µs | 4.69 ms | 47.1 ms | **~21.3 K/s** |
| WriteProperty | 468 µs | 4.71 ms | 47.3 ms | **~21.1 K/s** |

### 1.5 BACnet/SC — Mutual TLS (mTLS)

#### Latency

| Operation | Latency |
|---|---|
| ReadProperty | **47.1 µs** |
| WriteProperty | **46.8 µs** |
| RPM (10 objects) | **53.0 µs** |

#### Throughput

| Operation | 10 ops | 100 ops | 1000 ops | Peak ops/s |
|---|---|---|---|---|
| ReadProperty | 472 µs | 4.75 ms | 47.1 ms | **~21.2 K/s** |
| WriteProperty | 471 µs | 4.72 ms | 47.1 ms | **~21.2 K/s** |

---

## 2. Stress Tests

All stress tests run via:
```bash
cargo run --release -p bacnet-benchmarks --bin stress-test -- <scenario> [OPTIONS]
```

### 2.1 Concurrent Clients (`clients`)

Spawns N clients each doing continuous ReadProperty against a 100-object server.

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 1 | 12 µs | 28 µs | 73,948/s | 0 |
| 5 | 42 µs | 86 µs | 111,013/s | 0 |
| 10 | 80 µs | 170 µs | **118,249/s** | 0 |
| 25 | 244 µs | 467 µs | 100,595/s | 0 |
| 50 | 539 µs | 1,033 µs | 91,151/s | 0 |

Peak RSS: 12.2 MB. Throughput peaks at 10 clients (118K/s) with sub-millisecond p99 across all levels.

### 2.2 Object Database Scale (`objects`)

ReadProperty on random objects from databases of increasing size.

| Objects | RP p50 | RP p99 | RP Throughput | RPM p50 | RSS |
|---|---|---|---|---|---|
| 100 | 19 µs | 33 µs | 49,364/s | 25 µs | 8.9 MB |
| 500 | 19 µs | 32 µs | 49,345/s | 24 µs | 9.5 MB |
| 1,000 | 19 µs | 32 µs | 49,100/s | 24 µs | 9.8 MB |
| 2,500 | 19 µs | 33 µs | 48,247/s | 25 µs | 10.5 MB |
| 5,000 | 19 µs | 32 µs | **48,897/s** | 24 µs | 11.6 MB |

**No degradation** from 100 → 5,000 objects. RSS grows ~0.5 KB per object.

### 2.3 COV Subscription Saturation (`cov`)

Writes at 10 Hz, verifies all subscribers receive notifications.

| Subscriptions | Writes | Notifications Received | Delivery |
|---|---|---|---|
| 1 | 49 | 49/49 | **100%** |
| 5 | 49 | 49/49 | **100%** |
| 10 | 49 | 49/49 | **100%** |
| 25 | 50 | 50/50 | **100%** |

100% notification delivery across all subscription counts.

### 2.4 Segmented RPM (`segmentation`)

ReadPropertyMultiple with increasing numbers of objects per request.

| Objects/RPM | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 10 | 95 µs | 359 µs | 10,526/s | 0 |
| 25 | 93 µs | 143 µs | 10,753/s | 0 |
| 50 | 93 µs | 121 µs | 10,753/s | 0 |

Larger RPMs show stable tail latency (p99 converges).

### 2.5 Mixed Workload (`mixed`)

Weighted mix: 60% RP, 15% WP, 10% RPM, 5% COV, 5% WhoIs, 5% Device RP.

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 5 | 43 µs | 90 µs | **112,250/s** | 0 |

Peak RSS: 9.9 MB. Realistic building-automation workload at >112K ops/s.

### 2.6 Router Baseline (`router`)

Multi-client ReadProperty through direct (non-routed) path — establishes baseline for Docker cross-network comparison.

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 1 | 12 µs | 25 µs | 77,245/s | 0 |
| 3 | 30 µs | 59 µs | 93,123/s | 0 |
| 5 | 43 µs | 86 µs | **110,510/s** | 0 |

Peak RSS: 9.3 MB. Near-linear scaling to 5 clients.

### 2.7 BBMD Foreign Device (`bbmd`)

Foreign device registration + continuous broadcast through BBMD.

| Foreign Devices | p50 | p99 | Broadcast Rate | Errors |
|---|---|---|---|---|
| 1 | 18 µs | 32 µs | 794/s | 0 |
| 3 | 18 µs | 30 µs | 796/s | 0 |

Peak RSS: 7.7 MB. Broadcast rate is limited by the 200ms re-registration interval.

### 2.8 Device Scan (`whois`)

RP-based device scan across N servers (WhoIs broadcast is localhost-limited).

| Devices | Scan Time | p50 | p99 | All Found |
|---|---|---|---|---|
| 3 | <1 ms | 67 µs | 305 µs | 3/3 |
| 10 | <1 ms | 47 µs | 69 µs | 10/10 |
| 25 | 2 ms | 100 µs | 162 µs | 25/25 |

Peak RSS: 9.0 MB. Scan time scales linearly.

---

## 3. Transport Comparison Summary

### Rust (Criterion, single-threaded)

| Transport | RP Latency | RP Throughput | Overhead vs BIP |
|---|---|---|---|
| **BIP (UDP/IPv4)** | 20.9 µs | 48.3 K/s | — |
| **BIP6 (UDP/IPv6)** | 21.0 µs | 47.4 K/s | +0.5% latency |
| **SC (TLS WebSocket)** | 46.6 µs | 21.3 K/s | +123% latency |
| **SC mTLS** | 47.1 µs | 21.2 K/s | +125% latency |

SC mTLS adds negligible overhead vs server-auth-only SC — the TLS handshake dominates, not per-message client cert verification.

---

## 4. Docker Cross-Network Tests

> Platform: Alpine (aarch64 musl) | Docker Desktop | Static release binaries

### 4.1 Topology

5 Docker bridge networks, 9 services:
- **bacnet-a** (172.20.0.0/24): server-a (1000 objects), bbmd-a, router, foreign-client
- **bacnet-b** (172.20.1.0/24): server-b (1000 objects), bbmd-b, router
- **bacnet-sc** (172.20.3.0/24): sc-hub, sc-server (500 objects)
- **foreign** (172.20.4.0/24): foreign-client (also on bacnet-a for BBMD registration)

### 4.2 Cross-Network Orchestrator Results

| Scenario | Ops | Duration | Throughput | p50 | p99 | Errors |
|---|---|---|---|---|---|---|
| **Same-Subnet Baseline** | 84,603 | 5 s | 16,920 /s | 57 µs | 103 µs | 0 |
| **Router Hop** (subnet A → B) | 79,073 | 5 s | 15,814 /s | 61 µs | 123 µs | 0 |
| **Foreign Device** (via BBMD) | 82,156 | 5 s | 16,431 /s | 60 µs | 101 µs | 0 |

Router hop adds **~4 µs** (7%) vs same-subnet. Foreign device via BBMD adds **~3 µs** (5%). All 245,832 requests succeeded.

### 4.3 In-Container Stress Tests (self-contained, loopback)

#### Clients (max concurrent)

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 1 | 4 µs | 7 µs | 206,233 /s | 0 |
| 10 | 76 µs | 179 µs | 122,850 /s | 0 |
| 25 | 189 µs | 388 µs | 126,721 /s | 0 |
| 50 | 363 µs | 677 µs | 134,666 /s | 0 |

Peak RSS: 5,016 KB

#### Object Scale

| Objects | RP p50 | RP p99 | Throughput | RSS |
|---|---|---|---|---|
| 100 | 29 µs | 48 µs | 32,985 /s | 3,040 KB |
| 1,000 | 41 µs | 204 µs | 18,533 /s | 3,500 KB |
| 5,000 | 57 µs | 722 µs | 11,798 /s | 5,224 KB |
| 10,000 | 40 µs | 100 µs | 21,250 /s | 7,412 KB |

~440 bytes per object. Throughput variance at 5K objects due to container memory pressure.

#### COV Subscriptions

| Subscriptions | Writes | Notifications | Errors |
|---|---|---|---|
| 1 | 49 | 49/49 | 0 |
| 10 | 49 | 49/49 | 0 |
| 50 | 49 | 49/49 | 0 |
| 100 | 49 | 49/49 | 0 |

100% notification delivery at all subscription levels.

#### Segmented RPM

| Properties | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 10 | 53 µs | 193 µs | 18,868 /s | 0 |
| 25 | 57 µs | 87 µs | 17,544 /s | 0 |
| 50 | 67 µs | 115 µs | 14,925 /s | 0 |

#### Mixed Workload (RP + WP + RPM + WhoIs)

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 10 | 91 µs | 246 µs | 102,642 /s | 0 |

Peak RSS: 3,080 KB

#### Router Forwarding

| Clients | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 1 | 4 µs | 6 µs | 207,517 /s | 0 |
| 5 | 36 µs | 94 µs | 124,978 /s | 0 |
| 10 | 82 µs | 194 µs | 114,002 /s | 0 |
| 25 | 214 µs | 485 µs | 101,765 /s | 0 |

#### BBMD Foreign Device

| Foreign Devices | p50 | p99 | Throughput | Errors |
|---|---|---|---|---|
| 1 | 18 µs | 46 µs | 503 /s | 0 |
| 5 | 10 µs | 39 µs | 504 /s | 0 |
| 10 | 5 µs | 29 µs | 507 /s | 0 |

#### WhoIs Broadcast

| Devices | p50 | p99 | Discovery | Errors |
|---|---|---|---|---|
| 5 | 47 µs | 59 µs | 5/5 | 0 |
| 10 | 39 µs | 47 µs | 10/10 | 0 |
| 25 | 44 µs | 99 µs | 25/25 | 0 |

---

## 5. Python ↔ Rust Mixed-Mode Benchmarks

> Python 3.13.1 | PyO3 0.28 | `rusty_bacnet` wheel (release mode) | pytest + custom async harness
>
> All tests use the `rusty_bacnet` PyO3 bindings on localhost. SC benchmarks route
> through a Rust `ScHub` (TLS WebSocket relay) exposed via the `ScHub` Python class.

### 5.1 BACnet/IP — Python Client → Rust Server

| Operation | Mean | Median | p95 | p99 |
|---|---|---|---|---|
| ReadProperty | **113 µs** | 112 µs | 135 µs | 144 µs |
| WriteProperty | **71 µs** | 73 µs | 85 µs | 93 µs |
| RPM (3×2) | **70 µs** | 65 µs | 87 µs | 93 µs |
| COV Sub/Unsub | **125 µs** | 118 µs | 156 µs | 161 µs |

Sequential throughput: **12,704 ops/s** (ReadProperty)

| Concurrency | Throughput | p50 | p99 |
|---|---|---|---|
| 1 | 3,796 /s | 191 µs | 913 µs |
| 5 | 9,424 /s | 306 µs | 7,245 µs |
| 10 | 22,689 /s | 345 µs | 1,091 µs |
| 25 | **35,136 /s** | 542 µs | 1,663 µs |

Memory: 49.9 MB RSS (stable under load, +0.0 MB delta after 500 ops)

### 5.2 BACnet/IP — Rust Client → Python Server

| Operation | Mean | Median | p95 | p99 |
|---|---|---|---|---|
| ReadProperty | **116 µs** | 114 µs | 149 µs | 165 µs |
| WriteProperty | **75 µs** | 74 µs | 86 µs | 91 µs |
| RPM (3×2) | **83 µs** | 82 µs | 94 µs | 102 µs |
| COV Sub/Unsub | **151 µs** | 150 µs | 165 µs | 205 µs |

Sequential throughput: **13,233 ops/s** (ReadProperty)

| Concurrency | Throughput | p50 | p99 |
|---|---|---|---|
| 1 | 3,704 /s | 228 µs | 1,007 µs |
| 5 | 13,586 /s | 227 µs | 4,012 µs |
| 10 | 25,401 /s | 311 µs | 995 µs |
| 25 | **34,903 /s** | 580 µs | 1,498 µs |

Memory: 49.9 MB RSS (stable, +0.0 MB delta)

### 5.3 BACnet/IP — Python Client → Python Server

| Operation | Mean | Median | p95 | p99 |
|---|---|---|---|---|
| ReadProperty | **108 µs** | 108 µs | 126 µs | 134 µs |
| WriteProperty | **76 µs** | 75 µs | 86 µs | 93 µs |
| RPM (3×2) | **83 µs** | 81 µs | 95 µs | 104 µs |
| COV Sub/Unsub | **150 µs** | 148 µs | 169 µs | 221 µs |

Sequential throughput: **13,077 ops/s** (ReadProperty)

| Concurrency | Throughput | p50 | p99 |
|---|---|---|---|
| 1 | 3,929 /s | 210 µs | 825 µs |
| 5 | 14,983 /s | 216 µs | 3,388 µs |
| 10 | 28,044 /s | 282 µs | 782 µs |
| 25 | **36,163 /s** | 554 µs | 1,559 µs |

Memory: 49.8 MB RSS (stable, +0.0 MB delta)

### 5.4 BACnet/SC — Python Client → Rust Server (via ScHub)

| Operation | Mean | Median | p95 | p99 |
|---|---|---|---|---|
| ReadProperty | **226 µs** | 234 µs | 318 µs | 358 µs |
| WriteProperty | **110 µs** | 109 µs | 121 µs | 144 µs |
| RPM (3×2) | **111 µs** | 109 µs | 131 µs | 167 µs |
| COV Sub/Unsub | **206 µs** | 200 µs | 247 µs | 269 µs |

Sequential throughput: **7,054 ops/s** (ReadProperty)

| Concurrency | Throughput | p50 | p99 |
|---|---|---|---|
| 1 | 7,306 /s | 115 µs | 135 µs |
| 5 | 21,905 /s | 185 µs | 305 µs |
| 10 | 25,542 /s | 318 µs | 540 µs |
| 25 | **29,317 /s** | 646 µs | 1,227 µs |

Memory: 52.0 MB RSS (stable, +0.0 MB delta after 500 ops)

### 5.5 Python API Overhead Analysis

| Transport | Rust Latency | Python Latency | Overhead |
|---|---|---|---|
| BIP ReadProperty | 20.9 µs | ~110 µs | ~89 µs (+5.3×) |
| BIP WriteProperty | 21.1 µs | ~74 µs | ~53 µs (+3.5×) |
| SC ReadProperty | 46.6 µs | ~226 µs | ~179 µs (+4.8×) |
| SC WriteProperty | 47.3 µs | ~110 µs | ~63 µs (+2.3×) |

Python overhead is dominated by the asyncio event loop and PyO3 FFI boundary crossing (~50–90 µs).
Write operations show less overhead because the server doesn't need to encode a response payload.
Concurrent throughput scales well — 25 concurrent BIP clients reach **36K ops/s** from Python,
comparable to pure Rust's single-threaded 48K/s.

---

## 6. Key Takeaways

- **Encoding is fast**: Full RP encode/decode stack in ~82 ns (CPU-bound, no allocation hot paths thanks to `Bytes` zero-copy)
- **BIP throughput at ~48K/s**: Per-request task spawning enables concurrent `db.read()` — reads and writes now achieve equal throughput
- **BIP6 matches BIP**: IPv6 adds <1% latency overhead vs IPv4 — effectively identical performance
- **SC overhead is ~2.2×**: TLS WebSocket adds ~26 µs per operation vs raw UDP — acceptable for secure deployments
- **Object count doesn't matter**: 100 → 5,000 objects shows zero latency degradation (19 µs p50 across all sizes)
- **COV is reliable**: 100% notification delivery at 25 concurrent subscriptions (native) and 100 (Docker)
- **Zero errors across all tests**: No timeouts, no panics, no dropped messages
- **Docker validates real networking**: Cross-container BIP, routing, BBMD foreign device, and SC all work correctly
- **Minimal router/BBMD overhead**: Cross-subnet routing adds ~7% latency; BBMD foreign device adds ~5%
- **Musl/Alpine single-client 4 µs p50**: In-container loopback latency is extremely low, 206K ops/s single-threaded
- **Python API is production-ready**: ~80 µs PyO3 overhead per call; 36K concurrent ops/s from Python
- **SC from Python works**: ScHub + SC client/server all work via PyO3; 29K ops/s at 25 concurrent clients

---

## 7. How to Reproduce

```bash
# Criterion benchmarks (all 9 suites — run sequentially for accurate results)
cargo bench -p bacnet-benchmarks --bench encoding
cargo bench -p bacnet-benchmarks --bench bip_latency
cargo bench -p bacnet-benchmarks --bench bip_throughput
cargo bench -p bacnet-benchmarks --bench bip6_latency
cargo bench -p bacnet-benchmarks --bench bip6_throughput
cargo bench -p bacnet-benchmarks --bench sc_latency
cargo bench -p bacnet-benchmarks --bench sc_throughput
cargo bench -p bacnet-benchmarks --bench sc_mtls_latency
cargo bench -p bacnet-benchmarks --bench sc_mtls_throughput

# Quick run (reduced samples, ~10s per suite instead of ~60s)
cargo bench -p bacnet-benchmarks --bench bip_latency -- --sample-size 10 --warm-up-time 1

# Stress tests
cargo run --release -p bacnet-benchmarks --bin stress-test -- clients --steps 1,5,10,25,50 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- objects --steps 100,500,1000,2500,5000 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- cov --steps 1,5,10,25 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- segmentation --steps 10,25,50
cargo run --release -p bacnet-benchmarks --bin stress-test -- mixed --clients 5 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- router --steps 1,3,5 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- bbmd --steps 1,3 --duration 5
cargo run --release -p bacnet-benchmarks --bin stress-test -- whois --steps 3,10,25 --duration 5

# Docker cross-network (requires Docker)
cd examples/docker
docker compose build && docker compose up -d
docker compose exec orchestrator stress-orchestrator --duration 5
docker compose down

# Python mixed-mode benchmarks (requires Python 3.13+ and uv)
cd benchmarks/python
uv sync
uv run maturin develop --release --manifest-path ../../crates/rusty-bacnet/Cargo.toml
uv run pytest bench_py_client_rust_server.py -v   # BIP: Py client → Rust server
uv run pytest bench_rust_client_py_server.py -v   # BIP: Rust client → Py server
uv run pytest bench_py_py.py -v                   # BIP: Py ↔ Py
uv run pytest bench_sc.py -v                      # SC: Py client → Rust server via ScHub
```
