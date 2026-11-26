---
section: Added
---
- **Breaking (Rust API), Python API:** the routed-path summaries count answers
  refused for the path they came by: `RoutedPaths::off_path_answer_drops` in
  `bacnet-ops`, the CLI summary, MCP `bacnet://session`, and Python
  `BACnetClient.off_path_answer_drops()` (#2831).
