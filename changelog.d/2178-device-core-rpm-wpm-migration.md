---
section: Migration notes
---
- **Device core RPM and WPM (Rust API, #2178):** for
  `handlers::handle_read_property_multiple`, use `Responder::read` or
  `read::RpmPlan` with a budget. For
  `handlers::handle_write_property_multiple`, use `Responder::write`, or
  `write::write_property_multiple` then `write::end_unmade`.
  `ReadFailure::unlimited`, `read_target_object`, `rpm_response_index` and
  `read::group::plan_property` are no longer public.
