---
section: Migration notes
---
- **Property value encoders (Rust API, #2376):** `encode_bacnet_property_value`
  and the `encode` methods of `ReadPropertyACK`, `ReadPropertyMultipleACK`,
  `ReadAccessResult`, `ReadResultElement`, `CreateObjectRequest` and
  `COVNotificationRequest` return `Result<(), Error>`: propagate or handle it.
  `ReadPropertyAckRef::encode_to_sink` fails with
  `bacnet_encoding::sink::SinkEncodeError`, whose `Full` holds the old
  `SinkFull`.
