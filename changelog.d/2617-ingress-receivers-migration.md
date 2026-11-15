---
section: Migration notes
---
- **Endpoint ingress (Rust API, #2617):** `IngressReceivers` is `#[non_exhaustive]` with a new field.
  Code outside `bacnet-endpoint-core` that destructures it ends the pattern with `..`; it was never
  built outside the crate.
