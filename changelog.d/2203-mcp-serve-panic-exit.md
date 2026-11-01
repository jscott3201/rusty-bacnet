---
section: Changed
---
- `bacnet mcp serve` now exits with an error when a request's handler
  panics, after answering that request with an internal error, instead of
  leaving the agent waiting and serving on (#2203).
