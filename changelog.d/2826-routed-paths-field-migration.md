---
section: Migration notes
---
- **Routed-path summary (Rust API, #2826):** `bacnet_ops::client::RoutedPaths`
  has a new field, `network_number_disagreements`; add it to any struct
  expression that builds one, or build it with `..` from another value.
