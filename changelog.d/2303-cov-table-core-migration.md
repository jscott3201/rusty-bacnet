---
section: Migration notes
---
- **COV table (Rust API, #2303):** import `CovPolicy`, `CovCounters`,
  `CovRecipient`, `CovSubscription`, `CovSubscriptionKey`, `CovSample`,
  `CovObservation` and the other subscription types from
  `bacnet_device_core::cov`; `bacnet_server::server` no longer re-exports
  `CovPolicy` or `CovCounters`. `CovSubscription::expires_at` is a
  `bacnet_time::Instant`.
