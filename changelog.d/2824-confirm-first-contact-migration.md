---
section: Migration notes
---
- **First contact by instance (Rust, Python, CLI, MCP, #2824):** `device(n)`
  sends one Who-Is and asks each claim, and may fail as `OpsError::NotConfirmed`
  (`BacnetLookupError`), `Ambiguous` or `TableFull`; match `NotConfirmed {
  answer, placing, .. }`. `place_instance` takes a `Gate` (`&|_| None` for
  none) and returns `Result<Placement, Withheld>`; the desk's `Caller` gains
  `claim_refusal` and `unsent_text`. MCP refuses as `unconfirmed-device`.
