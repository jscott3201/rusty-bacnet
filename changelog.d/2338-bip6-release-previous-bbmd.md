---
section: Added
---
- **Wire:** A B/IPv6 foreign device can ask the BBMD it moved away from to delete its entry, as B/IP does (`Bip6Transport::set_release_previous_bbmd`, counted in `previous_bbmd_releases`), and `Bip6ClientBuilder` gains `release_previous_bbmd` and `bbmd_resolver` (#2338).
