---
section: Changed
---
- **Breaking (Rust API, Python API):** a first contact by instance costs
  about half a second and a read a claim (about 3 s when nothing answers);
  `OpsError::NotConfirmed` gains `answer` and `placing`, and a new handle
  never moves the handles already following the device (#2824).
