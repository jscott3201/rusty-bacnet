---
section: Changed
---
- **Breaking (Rust API):** `TsmResponse::NetworkPathTooLong` is replaced by
  `TsmResponse::Failed`, which holds the error a request ends with when its
  router refuses it or the device sends the wrong kind of acknowledgement,
  such as `Error::RoutedPathTooLong` (#2724, #2721).
