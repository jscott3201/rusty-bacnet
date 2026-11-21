---
section: Migration notes
---
- **Rust and Python API (#2612).** Add `dest_address` to any `DecodedNpdu` literal and read `IAmArrival::source` where you read `source_network`. Pickle `IAmArrival` records again with this wheel; older pickles do not load.
