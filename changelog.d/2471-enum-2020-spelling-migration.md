---
section: Migration notes
---
- **Enum spellings (Rust and Python API, #2471):** rename
  `ErrorCode::INVALID_TIME_STAMP` to `INVALID_TIMESTAMP` (Python
  `ErrorCode.INVALID_TIMESTAMP`) and `LifeSafetyMode::PRE_ARMED` to
  `PREARMED`. Output that printed `invalid-time-stamp` or `pre-armed` now
  prints `invalid-timestamp` or `prearmed`. `FromStr` and names typed into the
  CLI, TUI or MCP server ignore separators, so the old spellings still parse
  there.
