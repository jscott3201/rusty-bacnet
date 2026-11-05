---
section: Migration notes
---
- **Written configuration (Rust API, #2364):** `TimerWriteConfig`,
  `ChannelWriteConfig` and `ReportingWriteConfig` gain a `description`
  field, and the first two are now `#[non_exhaustive]`: build them from
  `Default` and set their fields. A file holding a written Description is
  kind 5 of `RBNWCF01`, which older builds refuse; files without one keep
  their octets.
