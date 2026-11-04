---
section: Migration notes
---
- **Property value boundaries (Rust API, #2376):**
  `bacnet_encoding::constructed::PropertyValueBoundary` gains
  `OneOctetContext(u8)`, a one-octet context tag followed by the end of the
  input or a context tag 0. Add an arm to any exhaustive match on it.
