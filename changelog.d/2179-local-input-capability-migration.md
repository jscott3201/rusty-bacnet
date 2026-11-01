---
section: Migration notes
---
- **LocalInput (Rust API, #2179):** replace `set_present_value_internal(v)`
  and its siblings with `impl LocalInput` (return it from `local_input()`):
  match `LocalInputValue::PresentValue { value, .. }` and the rest, answering
  others with `unsupported_local_input()`. Callers pass
  `LocalInputValue::present_value(v)` or `present_value_from(v, origin)`.
  `apply_life_safety_operation` moves to `impl LifeSafety`, returned from
  `life_safety()` ([Capabilities](docs/rust-api.md#capabilities)).
