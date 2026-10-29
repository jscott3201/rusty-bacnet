---
section: Migration notes
---
- **Alarms (#2077):** scripts reading `bacnet alarms` JSON take `summaries`
  and `more_events` instead of the hex `message`. Python callers of
  `get_event_information` read `result["summaries"]` instead of decoding
  bytes, and page with the last summary's `object_id` while
  `result["more_events"]` is true. Rust callers drop the
  `GetEventInformationAck::decode` call; an undecodable ACK is now the call's
  error.
