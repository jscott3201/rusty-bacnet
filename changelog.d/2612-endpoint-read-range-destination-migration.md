---
section: Migration notes
---
- **ClientRoleHandle (Rust API, #2612):** pass
  `read_range_with_destination` a `ReadRangeRequest { object_identifier,
  property_identifier, property_array_index, range }` and
  `ReadRangeValidation::Strict` in place of the four fields, and take `.ack`
  of the `ReadRangeReply` it returns for the `ReadRangeAck` it returned.
