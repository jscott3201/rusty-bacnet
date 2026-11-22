---
section: Migration notes
---
- **Routed path limits (Rust API, #2683):** `BACnetClient::configure_routed_path_max_npdu` and `clear_routed_path_limit` no longer return futures: drop the `.await`. Python calls are unchanged.
