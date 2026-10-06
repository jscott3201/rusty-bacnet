---
section: Changed
---
- **Breaking (Rust API):** `TransportPort::normal_bip_endpoint` is now
  `bip_port`, which reports the B/IP mode too; `BipTransport::bbmd_state`
  hands out a `std::sync::Mutex`, and `fdt_counters` no longer awaits (#939).
