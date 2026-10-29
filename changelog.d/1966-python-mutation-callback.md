---
section: Added
---
- **Python API:** `BACnetServer(mutation_policy=...)` also takes a callable
  that gets a redacted `MutationContext` and decides each covered mutation;
  exceptions and non-bool answers deny (#1966).
