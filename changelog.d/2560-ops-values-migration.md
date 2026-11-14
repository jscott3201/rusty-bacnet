---
section: Migration notes
---
- **bacnet-ops reads (Rust API, #2560):** `device.read(object, wanted)` is
  `read_object` and returns `Readings`; `read_property` is
  `read((object, property))`, a `Value` whose `raw` holds the octets; match
  `OpsError` instead of `Error`, with `e.unreachable()` and `e.timed_out()`;
  rename `RowError` to `PropertyError`.
  [Values, readings and OpsError](docs/rust-api.md#values-readings-and-opserror)
  has the table.
