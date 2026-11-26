---
section: Changed
---
- **Breaking (Python API):** an MS/TP serial port that fails to open raises
  `BacnetTransportError` (an `OSError`) naming the port, not `RuntimeError`;
  other transport I/O failures raise it instead of `BacnetError`; a send too
  long for the link reads `send refused before transmission: …` (#1833).
