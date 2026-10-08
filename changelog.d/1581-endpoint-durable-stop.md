---
section: Fixed
---
- Endpoint shutdown waits for queued durable saves and abandoned-write corrections before completing, including after cancellation and retry (#1581). Completion reports finished attempts, not storage success.
