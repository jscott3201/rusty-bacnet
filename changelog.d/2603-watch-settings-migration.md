---
section: Migration notes
---
- **Watch (Rust API, #2603):** `Lifetime::new` returns `Result` and refuses
  more than 28,800 s; `WatchError::Settings` holds a `Refusal`, and
  `WatchOptions::validate` returns `WatchError`; `WatchOptions` gains
  `subscribe`; `Watch::points` returns a `Vec`. Match `Why::PollOnly` and
  `WatchError::Closed`, and expect `None` from a watch whose hub shut down.
