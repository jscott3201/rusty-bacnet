---
section: Changed
---
- **Breaking (Rust API):** `ClientRoleHandle::read_range_with_destination`
  takes a `ReadRangeRequest` and a `ReadRangeValidation` and returns the
  `ReadRangeReply`, as `read_range_with` does, and the new
  `read_log_page_with_destination` pages a log at any destination (#2612).
