---
section: Changed
---
- **Breaking (Rust API):** a router application port's send after its router
  stopped fails with `Error::State` (`Closed`), so
  `OriginateFailure::from_error` returns `None` for it, where it returned
  `Some(OriginateFailure::Stopped)` (#1833).
