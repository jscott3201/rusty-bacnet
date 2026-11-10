---
section: Migration notes
---
- **Enum names (Rust and Python API, #2435):** rename `Segmentation::BOTH`,
  `TRANSMIT`, `RECEIVE`, `NONE` to `SEGMENTED_BOTH`, `SEGMENTED_TRANSMIT`,
  `SEGMENTED_RECEIVE`, `NO_SEGMENTATION`; `KILOHMS` and `MEGOHMS` to
  `KILOOHMS` and `MEGAOHMS`; `RESTART_AUTONEG` to `RESTART_AUTONEGOTIATION`;
  `LOG_ENABLE` to `ENABLE`; `COMMAND_NP` to `COMMAND`. Drop any use of
  `NetworkReachability`.
