---
section: Added
---
- **Rust API:** `RouterPort::isolated(true)` and the core's
  `PortConfig::isolated` mark a port; `Counters::isolated_drops` and
  `isolated_refusals`, `Discard::Isolated` and `Event::IsolationRefused`
  report what isolation stopped (#2684).
