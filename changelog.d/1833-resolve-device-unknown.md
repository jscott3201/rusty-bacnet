---
section: Changed
---
- **Breaking (Rust and Python API):** `BACnetClient::resolve_device` refuses an
  instance the table doesn't hold with `Error::UnknownDevice`, so the Python
  `_from_device` and `_to_device` calls raise `BacnetLookupError` for it, as
  documented, instead of `BacnetError` (#1833).
