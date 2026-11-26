---
section: Added
---
- **Breaking (Python API):** `BacnetStateError`, a `BacnetError` and a
  `RuntimeError` whose `state` names the lifecycle state, is raised for every
  state refusal: a stopped transport's send, which raised
  `BacnetTransportError`, and the others, which raised `BacnetError` (#1833).
