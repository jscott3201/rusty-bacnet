---
section: Changed
---
- **Breaking (Rust API):** `bacnet-ops` names command priorities with
  `Priority`: it replaces `label::priority_label` and `label::PRIORITIES`,
  the value grammar's `@N`, and `Effect::Unchanged`'s `held_by`, and
  `OpsError` gains `Denied` and `ReadBack` (#2560).
