---
section: Changed
---
- **Breaking (Rust API):** `BACnetRouter`'s control gate decides
  routing-table queries from another network: `ControlPolicy::Hardened`
  refuses them unless an authorizer allows, an authorizer is asked about
  each, and `ControlDecisionCounters` gains `init_query` (#2715).
