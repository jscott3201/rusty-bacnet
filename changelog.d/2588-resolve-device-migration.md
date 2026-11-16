---
section: Migration notes
---
- **Client by instance (Rust API, #2588):**
  `client.read_property_from_device(i, ..)` is
  `client.read_property(&client.resolve_device(i).await?, ..)`, and likewise
  for each `_from_device` and `_to_device` call. The lookup now comes first,
  so an unknown or contested instance is refused before an invalid
  WritePropertyMultiple, GetEventInformation, GetEnrollmentSummary or write
  priority is.
