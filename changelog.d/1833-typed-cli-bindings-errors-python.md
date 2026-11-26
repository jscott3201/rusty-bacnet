---
section: Changed
---
- **Python API:** an endpoint's second `start()` raises `BacnetStateError`
  (a `BacnetError`, `state` `"already_started"`); an empty SC credential
  path and a refused client stop keep their classes. None of these
  messages starts with `encoding error: ` any more (#1833).
