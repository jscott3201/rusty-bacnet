---
section: Changed
---
- **Breaking (Rust API):** `ReportingWriteConfig` gains `algorithm`, an
  `AlgorithmWriteConfig` of the written event algorithm parameters, stored
  as kind 6 of the `RBNWCF01` format; a file without one keeps kind 4
  (#2331).
