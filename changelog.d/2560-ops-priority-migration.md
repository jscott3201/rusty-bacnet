---
section: Migration notes
---
- **bacnet-ops priorities (Rust API, #2560):** `label::priority_label(n)` is
  `Priority::new(n)?.purpose()` and `label::PRIORITIES` is
  `Priority::COUNT`; `parse_value_with_priority` and
  `parse_value_for_with_priority` return `Option<Priority>` (`.get()` for
  the number); `bacnet_tool::safety::Effect` is `bacnet_ops::write::Effect`,
  its `held_by` an `Option<Priority>`, and it gains `Unknown`. Match
  `OpsError::Denied`, `ReadBack` and `NoPriorityArray` where you match
  every variant.
