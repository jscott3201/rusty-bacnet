---
section: Migration notes
---
- **Watch hubs (Rust API, #2561):** `watch::hub::Hub::new` returns `Result<Hub, WatchError>` instead of `Result<Hub, String>`: an invalid `WatchOptions` is `WatchError::Settings(reason)`, and a client whose 256 ranges are all held by live hubs `WatchError::Hubs`; drop a hub to free one. `Watch::next()` reads a watch without `futures_util::StreamExt`.
