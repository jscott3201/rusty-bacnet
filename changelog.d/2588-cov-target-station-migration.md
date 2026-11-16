---
section: Migration notes
---
- **Managed COV (Rust API, #2588):** `CovTarget` is `Station`:
  `CovTarget::local(mac)` is `Station::local(mac)`, `CovTarget::routed(r, n, a)`
  is `Station::routed(r, n, a)` and `CovTarget::of_device(&d)` is
  `Station::from(&d)`. `manage_cov_subscription` and
  `manage_cov_property_subscription` take `&station`; cancel with
  `unsubscribe_cov` or `unsubscribe_cov_property` to the same station in
  place of `unsubscribe_cov_target` and `unsubscribe_cov_property_target`.
