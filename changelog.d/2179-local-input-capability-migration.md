---
section: Migration notes
---
- **LocalInput, LifeSafety (Rust API, #2179):** `set_present_value_internal`
  and its siblings become `impl LocalInput` (lent by `local_input()`);
  `report_access_input_internal` is `LocalInputValue::AccessControl`.
  `apply_life_safety_operation` moves into `impl LifeSafety` (lent by
  `life_safety()`), and `set_life_safety_operation_expected_internal` is
  `LifeSafety::set_life_safety_operation_expected`. For
  `supports_access_event_batch_internal` use `access_events().is_some()`
  ([before and after](docs/rust-api.md#capabilities)).
