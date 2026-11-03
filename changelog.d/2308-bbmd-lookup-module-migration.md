---
section: Migration notes
---
- **BBMD host names (Rust API, #2308):** import `BbmdResolver`, `BbmdLookup` and `BbmdLookupFailure` from `bacnet_transport::bbmd_lookup`, not `bacnet_transport::bip`. A resolver returns `Vec<IpAddr>`: wrap IPv4 answers in `IpAddr::V4`. `BbmdLookupFailure::NoIpv4Address` is now `NoAddress`, and `Unusable` carries an `IpAddr`.
