---
section: Migration notes
---
- **Frame decoder (Rust API, #2612):** `bacnet_ops::decode::DecodedApdu::device` is
  replaced by `i_am`, the decoded `IAmRequest` (or why it failed): take the instance
  from `i_am.object_identifier`. `FrameSummary.device` is unchanged.
