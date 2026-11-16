---
section: Migration notes
---
- **Frame decoder (Rust API, #2612):** `bacnet_ops::decode::DecodedApdu::device` is
  replaced by `i_am: Option<Result<IAmRequest, String>>`, the decoded I-Am or why it
  failed: `apdu.i_am.and_then(Result::ok).map(|i_am| i_am.object_identifier.instance_number())`
  gives what `device` held. `FrameSummary.device` is unchanged.
