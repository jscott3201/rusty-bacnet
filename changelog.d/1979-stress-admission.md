---
section: Added
---
- `stress-test clients` takes `--requests` (a fixed number of requests per
  client instead of the time window) and, on B/IP, `--max-confirmed-in-flight`,
  and reports the server's peak of live confirmed handlers per step (#1979).
